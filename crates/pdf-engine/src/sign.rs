//! Digital signatures in PDF: create (`adbe.pkcs7.detached`, SHA-256) and inspect.
//!
//! Signing appends a new revision containing a signature field, its value dictionary with
//! reserved `/ByteRange` and `/Contents` space, and the AcroForm bookkeeping. The byte offsets are
//! only known once the revision is serialised, so the placeholders are patched in place and the
//! CMS signature (from the `pdf-sign` crate) is written into the reserved zero-filled hex string.
//!
//! What this module can and cannot say about a signature (see [`SignatureStatus`]): it checks the
//! digest and the signature mathematics with the certificate embedded in the signature. It does
//! **not** build or validate a certificate chain, check validity dates or revocation, or evaluate
//! DocMDP/FieldMDP permissions; a signature is therefore never reported as "trusted".

use crate::annot::{append_annot_ref, now_pdf_date};
use crate::doc::{PageId, PdfDocument, Tx};
use crate::error::{EngineError, Result};
use crate::fontembed::{BundledFace, FontBuilder, hex_string, zlib};
use crate::geom::Rect;
use crate::objutil::{self, fmt_num_prec, num_array, reference, text_obj};
use lopdf::{Dictionary, Object, ObjectId, Stream, StringFormat, dictionary};
pub use pdf_sign::Identity;

/// Bytes reserved for the CMS blob (hex-encoded in the file: twice this many characters).
pub const SIGNATURE_CAPACITY: usize = 16 * 1024;
const BR_MARK: &str = "9999999999";

/// Options for a new signature.
#[derive(Clone, Debug, Default)]
pub struct SignOptions {
    /// Free-text reason (`/Reason`).
    pub reason: String,
    /// Location (`/Location`).
    pub location: String,
    /// Contact information (`/ContactInfo`).
    pub contact: String,
    /// Page that holds the field. Visible signatures also give a rectangle (user space);
    /// `None` makes an invisible signature.
    pub page: Option<PageId>,
    /// Appearance rectangle on `page` (user space). `None` = invisible.
    pub rect: Option<Rect>,
}

/// Outcome of checking a signature that is already in a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureStatus {
    /// Digest matches and the signature verifies with the embedded certificate.
    IntegrityOk,
    /// The signed bytes were altered after signing (digest mismatch).
    DigestMismatch,
    /// The cryptographic signature itself does not verify.
    BadSignature,
    /// Could not be checked (reason).
    Unsupported(String),
}

/// A signature found in a document.
#[derive(Clone, Debug)]
pub struct SignatureInfo {
    /// Field name (`/T`).
    pub field: String,
    /// Signer common name from the embedded certificate, if readable.
    pub signer: String,
    /// Integrity result.
    pub status: SignatureStatus,
    /// Signing time claimed *by the signer* (not trusted time), if present.
    pub claimed_time: Option<String>,
    /// `/Reason`.
    pub reason: String,
    /// `/Location`.
    pub location: String,
    /// `/SubFilter`.
    pub sub_filter: String,
    /// Whether the signed revision is the end of the file (nothing appended since).
    pub covers_whole_file: bool,
    /// Page the field is on, if known.
    pub page: Option<PageId>,
}

fn catalog_id(doc: &lopdf::Document) -> Result<ObjectId> {
    Ok(doc.trailer.get(b"Root").and_then(Object::as_reference)?)
}

fn unique_field_name(doc: &lopdf::Document) -> String {
    let mut used = std::collections::BTreeSet::new();
    for o in doc.objects.values() {
        if let Object::Dictionary(d) = o
            && d.get(b"FT").ok().and_then(|f| f.as_name().ok()) == Some(b"Sig")
            && let Ok(t) = d.get(b"T")
            && let Ok(s) = t.as_str()
        {
            used.insert(objutil::decode_text_string(s));
        }
    }
    (1..)
        .map(|n| format!("Signature{n}"))
        .find(|n| !used.contains(n))
        .unwrap_or_else(|| "Signature".into())
}

fn register_in_acroform(tx: &mut Tx<'_>, field: ObjectId) -> Result<()> {
    let cat = catalog_id(tx.doc())?;
    let af = tx.doc().get_dictionary(cat)?.get(b"AcroForm").ok().cloned();
    let update = |d: &mut Dictionary| {
        let mut fields = d
            .get(b"Fields")
            .ok()
            .and_then(|o| o.as_array().ok())
            .cloned()
            .unwrap_or_default();
        fields.push(reference(field));
        d.set("Fields", Object::Array(fields));
        let flags = d
            .get(b"SigFlags")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0);
        d.set("SigFlags", flags | 3);
    };
    match af {
        Some(Object::Reference(r)) => update(tx.dict_mut(r)?),
        Some(Object::Dictionary(mut d)) => {
            update(&mut d);
            tx.dict_mut(cat)?.set("AcroForm", Object::Dictionary(d));
        }
        _ => {
            let mut d = Dictionary::new();
            update(&mut d);
            let id = tx.add(Object::Dictionary(d));
            tx.dict_mut(cat)?.set("AcroForm", reference(id));
        }
    }
    Ok(())
}

fn appearance(tx: &mut Tx<'_>, rect: Rect, lines: &[String]) -> Result<ObjectId> {
    let (w, h) = (rect.width(), rect.height());
    let mut fb = FontBuilder::new(BundledFace::Sans)?;
    // Characters the bundled font lacks are shown as '?' rather than failing a signature.
    let clean: Vec<String> = lines
        .iter()
        .map(|l| {
            l.chars()
                .map(|c| {
                    if c.is_control() || fb.glyph_for(c).is_some() {
                        c
                    } else {
                        '?'
                    }
                })
                .filter(|c| !c.is_control())
                .collect()
        })
        .collect();
    let widest = clean
        .iter()
        .map(|l| fb.text_width(l, 1.0))
        .fold(0.0f64, f64::max)
        .max(0.1);
    let n = clean.len().max(1) as f64;
    let fs = ((w - 8.0) / widest)
        .min((h - 6.0) / (n * 1.25))
        .clamp(3.0, 11.0);
    let mut c = format!(
        "q 0.35 g 0.5 w {} {} {} {} re S Q\nBT /F1 {} Tf 0 g\n",
        fmt_num_prec(0.25),
        fmt_num_prec(0.25),
        fmt_num_prec(w - 0.5),
        fmt_num_prec(h - 0.5),
        fmt_num_prec(fs)
    );
    let mut y = h - 3.0 - fs;
    for (i, l) in clean.iter().enumerate() {
        let codes = fb.encode_str(l)?;
        if i == 0 {
            c.push_str(&format!("{} {} Td\n", fmt_num_prec(4.0), fmt_num_prec(y)));
        } else {
            c.push_str(&format!("0 {} Td\n", fmt_num_prec(-fs * 1.25)));
        }
        if !codes.is_empty() {
            c.push_str(&format!("{} Tj\n", hex_string(&codes)));
        }
        y -= fs * 1.25;
    }
    c.push_str("ET\n");
    let font = fb.finish(tx)?;
    let mut s = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "FormType" => 1i64,
            "BBox" => num_array(&[0.0, 0.0, w, h]),
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            "Filter" => "FlateDecode",
        },
        zlib(c.as_bytes()),
    );
    s.allows_compression = false;
    Ok(tx.add(Object::Stream(s)))
}

fn pdf_date_to_text(d: &str) -> String {
    crate::meta::format_pdf_date(d) + " UTC"
}

impl PdfDocument {
    /// Add a signature field and sign the document. Returns the complete signed file bytes
    /// (the caller writes them out and then calls [`PdfDocument::rebase`]). Pending edits are
    /// part of what is signed. The document's in-memory objects are updated to match the bytes.
    pub fn sign(&mut self, identity: &Identity, opts: &SignOptions) -> Result<Vec<u8>> {
        if !self.capabilities().can_edit {
            return Err(EngineError::Unsupported(
                "this document cannot be modified, so it cannot be signed".into(),
            ));
        }
        let pages = self.page_ids()?;
        let page = opts
            .page
            .filter(|p| pages.contains(p))
            .or_else(|| pages.first().copied())
            .ok_or_else(|| EngineError::InvalidArgument("the document has no pages".into()))?;
        let date = now_pdf_date();
        let signer = identity.common_name().to_string();
        let ((value_id, _field_id), _) = self.transact(|tx| {
            let mut v = Dictionary::new();
            v.set("Type", Object::Name(b"Sig".to_vec()));
            v.set("Filter", Object::Name(b"Adobe.PPKLite".to_vec()));
            v.set("SubFilter", Object::Name(b"adbe.pkcs7.detached".to_vec()));
            v.set(
                "ByteRange",
                Object::Array(vec![
                    Object::Integer(0),
                    Object::Integer(9_999_999_999),
                    Object::Integer(9_999_999_999),
                    Object::Integer(9_999_999_999),
                ]),
            );
            v.set(
                "Contents",
                Object::String(vec![0u8; SIGNATURE_CAPACITY], StringFormat::Hexadecimal),
            );
            v.set("M", Object::string_literal(date.clone()));
            v.set("Name", text_obj(&signer));
            if !opts.reason.is_empty() {
                v.set("Reason", text_obj(&opts.reason));
            }
            if !opts.location.is_empty() {
                v.set("Location", text_obj(&opts.location));
            }
            if !opts.contact.is_empty() {
                v.set("ContactInfo", text_obj(&opts.contact));
            }
            let value_id = tx.add(Object::Dictionary(v));

            let name = unique_field_name(tx.doc());
            let mut w = Dictionary::new();
            w.set("Type", Object::Name(b"Annot".to_vec()));
            w.set("Subtype", Object::Name(b"Widget".to_vec()));
            w.set("FT", Object::Name(b"Sig".to_vec()));
            w.set("T", text_obj(&name));
            w.set("V", reference(value_id));
            w.set("F", 132i64); // Print + Locked
            w.set("P", reference(page.0));
            match opts
                .rect
                .map(|r| r.abs())
                .filter(|r| r.width() >= 20.0 && r.height() >= 10.0)
            {
                Some(r) => {
                    w.set("Rect", num_array(&[r.x0, r.y0, r.x1, r.y1]));
                    let mut lines = vec![format!("Digitally signed by {signer}")];
                    lines.push(format!("Date: {}", pdf_date_to_text(&date)));
                    if !opts.reason.is_empty() {
                        lines.push(format!("Reason: {}", opts.reason));
                    }
                    if !opts.location.is_empty() {
                        lines.push(format!("Location: {}", opts.location));
                    }
                    let ap = appearance(tx, r, &lines)?;
                    w.set("AP", dictionary! { "N" => ap });
                }
                None => w.set("Rect", num_array(&[0.0, 0.0, 0.0, 0.0])),
            }
            let field_id = tx.add(Object::Dictionary(w));
            append_annot_ref(tx, page.0, field_id)?;
            register_in_acroform(tx, field_id)?;
            Ok((value_id, field_id))
        })?;

        let mut bytes = self.snapshot_bytes()?;
        let region = if self.can_save_incrementally() {
            self.original_bytes().len().min(bytes.len())
        } else {
            0
        };
        // `can_save_incrementally` describes the state *before* the snapshot above; for a
        // rewritten file the whole buffer is the region.
        patch_signature(&mut bytes, region, identity)?;
        // Keep the in-memory graph consistent with the bytes we are about to write.
        self.caps.has_signatures = true;
        let (br, contents) = read_back(&bytes, value_id)?;
        if let Some(Object::Dictionary(d)) = self.lopdf_mut().objects.get_mut(&value_id) {
            d.set(
                "ByteRange",
                Object::Array(br.iter().map(|n| Object::Integer(*n)).collect()),
            );
            d.set(
                "Contents",
                Object::String(contents, StringFormat::Hexadecimal),
            );
        }
        Ok(bytes)
    }
}

/// Locate the placeholders, compute the digest over the byte ranges, embed the CMS blob.
fn patch_signature(bytes: &mut [u8], region_start: usize, identity: &Identity) -> Result<()> {
    let zeros = vec![b'0'; SIGNATURE_CAPACITY * 2];
    // 1. The reserved Contents string: `<` + zeros + `>` inside the new revision (last match).
    let mut hex_start = None;
    let mut from = region_start;
    while let Some(p) = find(bytes, b"/Contents", from) {
        let mut q = p + b"/Contents".len();
        while q < bytes.len() && bytes[q].is_ascii_whitespace() {
            q += 1;
        }
        if bytes.get(q) == Some(&b'<')
            && bytes.get(q + 1..q + 1 + zeros.len()) == Some(zeros.as_slice())
            && bytes.get(q + 1 + zeros.len()) == Some(&b'>')
        {
            hex_start = Some(q);
        }
        from = p + 1;
    }
    let lt =
        hex_start.ok_or_else(|| EngineError::Save("signature placeholder not found".into()))?;
    let gt = lt + 1 + zeros.len(); // index of '>'
    // 2. The ByteRange placeholder (the last one in the file).
    let mut br_at = None;
    let mut from = region_start;
    let marker = format!("{BR_MARK} {BR_MARK} {BR_MARK}");
    while let Some(p) = find(bytes, marker.as_bytes(), from) {
        br_at = Some(p);
        from = p + 1;
    }
    let br = br_at.ok_or_else(|| EngineError::Save("ByteRange placeholder not found".into()))?;
    let total = bytes.len();
    let (a, b) = (lt, gt + 1);
    let c = total - b;
    let fixed = format!("{a:010} {b:010} {c:010}");
    bytes[br..br + fixed.len()].copy_from_slice(fixed.as_bytes());
    // 3. Digest of everything except the Contents string, then the signature.
    let digest = pdf_sign::sha256(&[&bytes[..a], &bytes[b..]]);
    let cms = identity
        .sign_digest(&digest)
        .map_err(|e| EngineError::Unsupported(e.to_string()))?;
    if cms.len() > SIGNATURE_CAPACITY {
        return Err(EngineError::LimitExceeded(format!(
            "the signature ({} bytes) does not fit the reserved space; the certificate chain is too large",
            cms.len()
        )));
    }
    for (i, byte) in cms.iter().enumerate() {
        let hx = format!("{byte:02X}");
        bytes[lt + 1 + 2 * i..lt + 3 + 2 * i].copy_from_slice(hx.as_bytes());
    }
    Ok(())
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Read the final ByteRange and Contents bytes of the signature value `id` from signed bytes.
fn read_back(bytes: &[u8], id: ObjectId) -> Result<(Vec<i64>, Vec<u8>)> {
    let d = lopdf::Document::load_mem(bytes).map_err(|e| EngineError::Save(e.to_string()))?;
    let v = d
        .get_dictionary(id)
        .map_err(|_| EngineError::Save("signature value missing".into()))?;
    let br: Vec<i64> = v
        .get(b"ByteRange")
        .ok()
        .and_then(|o| o.as_array().ok())
        .map(|a| a.iter().filter_map(|o| o.as_i64().ok()).collect())
        .unwrap_or_default();
    let contents = v
        .get(b"Contents")
        .ok()
        .and_then(|o| o.as_str().ok())
        .map(<[u8]>::to_vec)
        .unwrap_or_default();
    Ok((br, contents))
}

/// DER SEQUENCE length (header + body) at the start of `data`, ignoring zero padding after it.
fn der_total_len(data: &[u8]) -> Option<usize> {
    if data.first() != Some(&0x30) {
        return None;
    }
    let l0 = *data.get(1)?;
    let (len, hdr) = if l0 & 0x80 == 0 {
        (usize::from(l0), 2)
    } else {
        let n = usize::from(l0 & 0x7F);
        if n == 0 || n > 4 {
            return None;
        }
        let mut v = 0usize;
        for i in 0..n {
            v = (v << 8) | usize::from(*data.get(2 + i)?);
        }
        (v, 2 + n)
    };
    let total = hdr.checked_add(len)?;
    (total <= data.len()).then_some(total)
}

/// Inspect every signature in the document bytes.
pub fn list_signatures(doc: &PdfDocument) -> Vec<SignatureInfo> {
    let bytes = doc.original_bytes();
    let d = doc.lopdf();
    let page_of = |field: ObjectId| -> Option<PageId> {
        doc.page_ids().ok()?.into_iter().find(|p| {
            d.get_dictionary(p.0)
                .ok()
                .and_then(|pd| objutil::dict_array(d, pd, b"Annots"))
                .is_some_and(|a| a.iter().any(|o| o.as_reference().ok() == Some(field)))
        })
    };
    let mut out = Vec::new();
    for (id, obj) in &d.objects {
        let Object::Dictionary(f) = obj else { continue };
        if f.get(b"FT").ok().and_then(|o| o.as_name().ok()) != Some(b"Sig") {
            continue;
        }
        let Some(v) = objutil::dict_dict(d, f, b"V") else {
            continue;
        };
        let text = |k: &[u8]| {
            v.get(k)
                .ok()
                .and_then(|o| objutil::deref(d, o))
                .and_then(|o| o.as_str().ok())
                .map(objutil::decode_text_string)
                .unwrap_or_default()
        };
        let name = f
            .get(b"T")
            .ok()
            .and_then(|o| o.as_str().ok())
            .map(objutil::decode_text_string)
            .unwrap_or_default();
        let sub_filter = v
            .get(b"SubFilter")
            .ok()
            .and_then(|o| o.as_name().ok())
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        let br: Vec<i64> = objutil::dict_array(d, v, b"ByteRange")
            .map(|a| {
                a.iter()
                    .filter_map(|o| objutil::num(d, o).map(|x| x as i64))
                    .collect()
            })
            .unwrap_or_default();
        let contents = v
            .get(b"Contents")
            .ok()
            .and_then(|o| o.as_str().ok())
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        let mut info = SignatureInfo {
            field: name,
            signer: text(b"Name"),
            status: SignatureStatus::Unsupported("not checked".into()),
            claimed_time: None,
            reason: text(b"Reason"),
            location: text(b"Location"),
            sub_filter: sub_filter.clone(),
            covers_whole_file: false,
            page: page_of(*id),
        };
        let valid_range = br.len() == 4
            && br[0] == 0
            && br.iter().all(|n| *n >= 0)
            && (br[2] + br[3]) as usize <= bytes.len()
            && (br[1] as usize) <= (br[2] as usize);
        if !valid_range {
            info.status =
                SignatureStatus::Unsupported("the signature's ByteRange is invalid".into());
            out.push(info);
            continue;
        }
        info.covers_whole_file = (br[2] + br[3]) as usize == bytes.len();
        if !sub_filter.contains("pkcs7.detached") && sub_filter != "ETSI.CAdES.detached" {
            info.status =
                SignatureStatus::Unsupported(format!("unsupported signature type {sub_filter}"));
            out.push(info);
            continue;
        }
        let (a, b, c) = (br[1] as usize, br[2] as usize, br[3] as usize);
        let digest = pdf_sign::sha256(&[&bytes[..a], &bytes[b..b + c]]);
        match der_total_len(&contents) {
            None => {
                info.status =
                    SignatureStatus::Unsupported("the signature data is unreadable".into())
            }
            Some(n) => match pdf_sign::verify_detached(&contents[..n], &digest) {
                Ok(v) => {
                    if info.signer.is_empty() {
                        info.signer = v.signer.clone();
                    }
                    info.claimed_time = v.claimed_time.clone();
                    info.status = if !v.digest_matches {
                        SignatureStatus::DigestMismatch
                    } else if !v.signature_valid {
                        SignatureStatus::BadSignature
                    } else {
                        SignatureStatus::IntegrityOk
                    };
                }
                Err(e) => info.status = SignatureStatus::Unsupported(e.to_string()),
            },
        }
        out.push(info);
    }
    out.sort_by(|x, y| x.field.cmp(&y.field));
    out
}
