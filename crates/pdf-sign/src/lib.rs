//! Detached CMS (PKCS#7) signatures for PDF documents.
//!
//! * [`Identity`] is a signing certificate chain plus private key loaded from a PKCS#12
//!   (`.p12` / `.pfx`) file. RSA (PKCS#1 v1.5) and ECDSA P-256 keys are supported; digest is
//!   SHA-256.
//! * [`Identity::sign_digest`] produces the DER `ContentInfo(SignedData)` that goes into a PDF
//!   signature's `/Contents` (`/SubFilter /adbe.pkcs7.detached`).
//! * [`verify_detached`] checks a signature's *mathematics* (the message digest attribute equals
//!   the supplied digest and the signature over the signed attributes verifies with the embedded
//!   signer certificate). It does **not** check certificate trust, validity period or revocation —
//!   callers must never present its result as "trusted".
//!
//! Private key material never leaves this crate's types and is not logged.

use cms::builder::{SignedDataBuilder, SignerInfoBuilder, create_signing_time_attribute};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::ContentInfo;
use cms::signed_data::{EncapsulatedContentInfo, SignedData, SignerIdentifier};
use der::asn1::{ObjectIdentifier, OctetString};
use der::{Decode, Encode};
use p12_keystore::{KeyStore, Pkcs12ImportPolicy};
use pkcs8::DecodePrivateKey;
use rsa::pkcs1v15::{
    Signature as RsaSignature, SigningKey as RsaSigningKey, VerifyingKey as RsaVerifyingKey,
};
use rsa::signature::Verifier;
use sha2::{Digest, Sha256};
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;

/// OID of `id-sha256`.
const ID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
/// OID of `id-data`.
const ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
/// OID of the `messageDigest` attribute.
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
/// OID of `commonName`.
const ID_CN: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");
/// OID of `rsaEncryption`.
const OID_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
/// OID of `id-ecPublicKey`.
const OID_EC: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");

/// Errors from loading identities and signing.
#[derive(Debug, thiserror::Error)]
pub enum SignError {
    /// The file is not a readable PKCS#12 store or the password is wrong.
    #[error(
        "could not open the certificate file: wrong password or not a PKCS#12 (.p12/.pfx) file"
    )]
    BadPkcs12,
    /// The store has no private key with a certificate.
    #[error("the certificate file contains no private key with a certificate")]
    NoPrivateKey,
    /// The key type is not supported.
    #[error("unsupported key type: {0} (RSA and ECDSA P-256 are supported)")]
    UnsupportedKey(String),
    /// Certificate could not be parsed.
    #[error("the certificate could not be read: {0}")]
    BadCertificate(String),
    /// Building the CMS structure failed.
    #[error("could not create the signature: {0}")]
    Cms(String),
}

enum Key {
    Rsa(Box<RsaSigningKey<Sha256>>),
    P256(Box<p256::ecdsa::SigningKey>),
}

/// A signing identity: private key + certificate chain (leaf first).
pub struct Identity {
    key: Key,
    certs: Vec<Certificate>,
    subject: String,
    common_name: String,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("subject", &self.subject)
            .field("certs", &self.certs.len())
            .finish_non_exhaustive()
    }
}

fn common_name_of(cert: &Certificate) -> Option<String> {
    for rdn in cert.tbs_certificate.subject.0.iter() {
        for atv in rdn.0.iter() {
            if atv.oid == ID_CN {
                if let Ok(s) = atv.value.decode_as::<der::asn1::Utf8StringRef<'_>>() {
                    return Some(s.as_str().to_string());
                }
                if let Ok(s) = atv.value.decode_as::<der::asn1::PrintableStringRef<'_>>() {
                    return Some(s.as_str().to_string());
                }
            }
        }
    }
    None
}

impl Identity {
    /// Load the first private key (with its certificate chain) from a PKCS#12 file.
    pub fn from_pkcs12(bytes: &[u8], password: &str) -> Result<Identity, SignError> {
        let store = KeyStore::from_pkcs12(bytes, password, Pkcs12ImportPolicy::Strict)
            .map_err(|_| SignError::BadPkcs12)?;
        let (_, chain) = store.private_key_chain().ok_or(SignError::NoPrivateKey)?;
        let certs: Vec<Certificate> = chain
            .certs()
            .iter()
            .map(|c| {
                Certificate::from_der(c.as_der())
                    .map_err(|e| SignError::BadCertificate(e.to_string()))
            })
            .collect::<Result<_, _>>()?;
        let leaf = certs.first().ok_or(SignError::NoPrivateKey)?;
        let pkcs8 = chain.key().as_der();
        // p12-keystore uses a newer `const-oid`; compare by text.
        let key_oid = chain.key().oid().to_string();
        let key = match key_oid.as_str() {
            o if o == OID_RSA.to_string() => {
                let k = rsa::RsaPrivateKey::from_pkcs8_der(pkcs8)
                    .map_err(|e| SignError::UnsupportedKey(e.to_string()))?;
                Key::Rsa(Box::new(RsaSigningKey::<Sha256>::new(k)))
            }
            o if o == OID_EC.to_string() => {
                let k = p256::ecdsa::SigningKey::from_pkcs8_der(pkcs8).map_err(|_| {
                    SignError::UnsupportedKey("elliptic-curve key that is not P-256".into())
                })?;
                Key::P256(Box::new(k))
            }
            o => return Err(SignError::UnsupportedKey(o.to_string())),
        };
        let subject = leaf.tbs_certificate.subject.to_string();
        let common_name = common_name_of(leaf).unwrap_or_else(|| subject.clone());
        Ok(Identity {
            key,
            certs,
            subject,
            common_name,
        })
    }

    /// Full subject distinguished name of the signing certificate.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Common name (or the whole subject when there is none).
    pub fn common_name(&self) -> &str {
        &self.common_name
    }

    /// Number of certificates that will be embedded in signatures.
    pub fn chain_len(&self) -> usize {
        self.certs.len()
    }

    /// DER `ContentInfo(SignedData)` for a detached signature over `digest` (SHA-256).
    pub fn sign_digest(&self, digest: &[u8; 32]) -> Result<Vec<u8>, SignError> {
        let leaf = self.certs.first().ok_or(SignError::NoPrivateKey)?;
        let content = EncapsulatedContentInfo {
            econtent_type: ID_DATA,
            econtent: None,
        };
        let digest_alg = AlgorithmIdentifierOwned {
            oid: ID_SHA256,
            parameters: None,
        };
        let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: leaf.tbs_certificate.issuer.clone(),
            serial_number: leaf.tbs_certificate.serial_number.clone(),
        });
        let cms_err = |e: &dyn std::fmt::Display| SignError::Cms(e.to_string());
        let mut builder = SignedDataBuilder::new(&content);
        builder
            .add_digest_algorithm(digest_alg.clone())
            .map_err(|e| cms_err(&e))?;
        for c in &self.certs {
            builder
                .add_certificate(CertificateChoices::Certificate(c.clone()))
                .map_err(|e| cms_err(&e))?;
        }
        match &self.key {
            Key::Rsa(k) => {
                let mut sib =
                    SignerInfoBuilder::new(k.as_ref(), sid, digest_alg, &content, Some(digest))
                        .map_err(|e| cms_err(&e))?;
                sib.add_signed_attribute(create_signing_time_attribute().map_err(|e| cms_err(&e))?)
                    .map_err(|e| cms_err(&e))?;
                builder
                    .add_signer_info::<RsaSigningKey<Sha256>, RsaSignature>(sib)
                    .map_err(|e| cms_err(&e))?;
            }
            Key::P256(k) => {
                let mut sib =
                    SignerInfoBuilder::new(k.as_ref(), sid, digest_alg, &content, Some(digest))
                        .map_err(|e| cms_err(&e))?;
                sib.add_signed_attribute(create_signing_time_attribute().map_err(|e| cms_err(&e))?)
                    .map_err(|e| cms_err(&e))?;
                builder
                    .add_signer_info::<p256::ecdsa::SigningKey, p256::ecdsa::DerSignature>(sib)
                    .map_err(|e| cms_err(&e))?;
            }
        }
        let ci = builder.build().map_err(|e| cms_err(&e))?;
        ci.to_der().map_err(|e| cms_err(&e))
    }
}

/// SHA-256 helper.
pub fn sha256(data: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for d in data {
        h.update(d);
    }
    h.finalize().into()
}

/// What [`verify_detached`] found out about a signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verification {
    /// Common name (or subject) of the signer certificate embedded in the signature.
    pub signer: String,
    /// The signature's message-digest attribute matches the document digest.
    pub digest_matches: bool,
    /// The signature over the signed attributes verifies with the embedded certificate.
    pub signature_valid: bool,
    /// Signing time claimed by the signer (`YYMMDDhhmmssZ`/GeneralizedTime text), if present.
    pub claimed_time: Option<String>,
}

/// Check the mathematics of a detached signature (no trust, expiry or revocation checks).
pub fn verify_detached(
    cms_der: &[u8],
    expected_digest: &[u8; 32],
) -> Result<Verification, SignError> {
    let bad = |m: &str| SignError::Cms(m.to_string());
    let ci = ContentInfo::from_der(cms_der).map_err(|e| SignError::Cms(e.to_string()))?;
    let sd: SignedData = ci
        .content
        .decode_as()
        .map_err(|e| SignError::Cms(e.to_string()))?;
    let si = sd
        .signer_infos
        .0
        .iter()
        .next()
        .ok_or_else(|| bad("no signer"))?;
    let certs: Vec<Certificate> = sd
        .certificates
        .as_ref()
        .map(|c| {
            c.0.iter()
                .filter_map(|ch| match ch {
                    CertificateChoices::Certificate(c) => Some(c.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let SignerIdentifier::IssuerAndSerialNumber(sid) = &si.sid else {
        return Err(bad("unsupported signer identifier"));
    };
    let signer_cert = certs
        .iter()
        .find(|c| {
            c.tbs_certificate.serial_number == sid.serial_number
                && c.tbs_certificate.issuer == sid.issuer
        })
        .ok_or_else(|| bad("signer certificate not embedded"))?;
    let signer = common_name_of(signer_cert)
        .unwrap_or_else(|| signer_cert.tbs_certificate.subject.to_string());
    let attrs = si
        .signed_attrs
        .as_ref()
        .ok_or_else(|| bad("signature has no signed attributes"))?;
    let md_attr = attrs
        .iter()
        .find(|a| a.oid == ID_MESSAGE_DIGEST)
        .and_then(|a| a.values.iter().next())
        .ok_or_else(|| bad("no message digest attribute"))?;
    let md = md_attr
        .decode_as::<OctetString>()
        .map_err(|e| SignError::Cms(e.to_string()))?;
    let digest_matches = md.as_bytes() == expected_digest;
    let claimed_time = attrs
        .iter()
        .find(|a| a.oid == ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5"))
        .and_then(|a| a.values.iter().next())
        .and_then(|v| {
            v.decode_as::<der::asn1::UtcTime>()
                .ok()
                .map(|t| t.to_date_time().to_string())
                .or_else(|| {
                    v.decode_as::<der::asn1::GeneralizedTime>()
                        .ok()
                        .map(|t| t.to_date_time().to_string())
                })
        });
    // The signature covers the DER encoding of the attributes as an explicit SET OF.
    let signed_bytes = attrs.to_der().map_err(|e| SignError::Cms(e.to_string()))?;
    let spki = &signer_cert.tbs_certificate.subject_public_key_info;
    let sig = si.signature.as_bytes();
    let signature_valid = match spki.algorithm.oid {
        o if o == OID_RSA => {
            use rsa::pkcs8::DecodePublicKey;
            let spki_der = spki.to_der().map_err(|e| SignError::Cms(e.to_string()))?;
            let pk = rsa::RsaPublicKey::from_public_key_der(&spki_der)
                .map_err(|e| SignError::Cms(e.to_string()))?;
            let vk = RsaVerifyingKey::<Sha256>::new(pk);
            RsaSignature::try_from(sig)
                .ok()
                .is_some_and(|s| vk.verify(&signed_bytes, &s).is_ok())
        }
        o if o == OID_EC => {
            use p256::pkcs8::DecodePublicKey as _;
            let spki_der = spki.to_der().map_err(|e| SignError::Cms(e.to_string()))?;
            let vk = p256::ecdsa::VerifyingKey::from_public_key_der(&spki_der)
                .map_err(|_| bad("unsupported elliptic curve"))?;
            p256::ecdsa::DerSignature::try_from(sig)
                .ok()
                .is_some_and(|s| vk.verify(&signed_bytes, &s).is_ok())
        }
        _ => return Err(bad("unsupported signer key type")),
    };
    Ok(Verification {
        signer,
        digest_matches,
        signature_valid,
        claimed_time,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn fx(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn loads_every_fixture_flavour() {
        for (file, cn, certs) in [
            ("rsa-aes.p12", "BergPDF Test RSA", 1),
            ("rsa-legacy3des.p12", "BergPDF Test RSA", 1),
            ("ec-aes.p12", "BergPDF Test EC", 1),
            ("chain-aes.p12", "Mark Test Signer", 2),
        ] {
            let id = Identity::from_pkcs12(&fx(file), "test123")
                .unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(id.common_name(), cn, "{file}");
            assert_eq!(id.chain_len(), certs, "{file}");
        }
    }

    #[test]
    fn wrong_password_and_garbage_are_refused() {
        assert!(matches!(
            Identity::from_pkcs12(&fx("rsa-aes.p12"), "nope"),
            Err(SignError::BadPkcs12)
        ));
        assert!(matches!(
            Identity::from_pkcs12(b"not a p12", "x"),
            Err(SignError::BadPkcs12)
        ));
    }

    #[test]
    fn rsa_and_ecdsa_signatures_verify_and_detect_tampering() {
        let digest = sha256(&[b"hello ", b"world"]);
        for file in [
            "rsa-aes.p12",
            "rsa-legacy3des.p12",
            "ec-aes.p12",
            "chain-aes.p12",
        ] {
            let id = Identity::from_pkcs12(&fx(file), "test123").unwrap();
            let sig = id.sign_digest(&digest).unwrap();
            let v = verify_detached(&sig, &digest).unwrap();
            assert!(v.digest_matches && v.signature_valid, "{file}: {v:?}");
            assert_eq!(v.signer, id.common_name());
            assert!(v.claimed_time.is_some());
            // A different document digest no longer matches.
            let other = sha256(&[b"hello worle"]);
            let t = verify_detached(&sig, &other).unwrap();
            assert!(!t.digest_matches, "{file}");
            // Flipping a byte inside the signature value breaks the maths.
            let mut broken = sig.clone();
            let n = broken.len();
            // The signature value sits near the end; avoid the trailing structure bytes.
            broken[n - 8] ^= 0x55;
            // (A structurally damaged signature is also a rejection.)
            if let Ok(b) = verify_detached(&broken, &digest) {
                assert!(
                    !(b.digest_matches && b.signature_valid),
                    "{file}: tampered signature accepted"
                );
            }
        }
    }
}
