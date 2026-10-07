# Distributing BergPDF through the Microsoft Store

What is in the repository for the Store, what only the owner can do in Partner Center, and what to check by hand.
Why it is built this way: [`DECISIONS.md`](DECISIONS.md) D-034.

## What the repository provides

* `cargo xtask msix` builds the **MSIX package** from the same release binary as the other installers
  (`xtask/src/msix.rs`: manifest, logos in all sizes, `resources.pri`, then `makeappx pack`).
* The package contains the program, `AppxManifest.xml`, the licence files and a marker file
  (`microsoft-store.marker`). With the marker the program knows it is the Store copy and **switches its own update
  check off** (no start-up question, no "Update available", no *Check for Updates…*, no *Updates* preference): the
  Store updates it, and Store policy forbids pointing people to another download. Nothing else differs.
* The manifest registers BergPDF as a handler for `.pdf` ("Open with", Default apps) and declares one capability,
  `runFullTrust` (see below). Languages listed: English, Dutch, German. Minimum Windows: 10 version 1809; x64 only.
* `.github/workflows/release.yml` builds the package on every release run and keeps it as the workflow artefact
  `store-package` (not on the GitHub release: it is unsigned and cannot be installed outside the Store).
* [`PRIVACY.md`](../PRIVACY.md) is the privacy policy the Store listing needs.

## One-time setup in Partner Center (owner)

1. Register a developer account at <https://partner.microsoft.com/dashboard> (individual or company; check the current
   fee and identity checks, which can take days for a company).
2. **Create the product:** *Apps and games ▸ New product ▸ MSIX or PWA app*, and reserve the name **BergPDF**. The name
   is free only if nobody else holds it, and it has **not been checked as a trademark** (D-013). Microsoft can remove
   an app over a trademark complaint, so do that check before the first submission.
3. Open *Product management ▸ Product identity*. It shows three values: **Package/Identity/Name**,
   **Package/Identity/Publisher** (`CN=…`) and **PublisherDisplayName**. The package must contain exactly these.
   Give them to the build, either once as repository variables in GitHub (*Settings ▸ Secrets and variables ▸
   Actions ▸ Variables*): `STORE_IDENTITY_NAME`, `STORE_PUBLISHER`, `STORE_PUBLISHER_DISPLAY_NAME`; or on the command
   line (below).
4. If the reserved name differs from "BergPDF", change `DISPLAY_NAME` in `xtask/src/msix.rs`.

## Building the package

```
winget install Microsoft.WindowsSDK.10.0.26100      # once: makeappx and makepri
cargo xtask msix                                    # test package with a placeholder identity
cargo xtask msix --store --identity-name <Name> --publisher "<CN=…>" --publisher-display-name "<Display name>"
```

The environment variables `BERG_STORE_IDENTITY_NAME`, `BERG_STORE_PUBLISHER` and `BERG_STORE_PUBLISHER_DISPLAY_NAME`
do the same as the flags. Without an identity the file is called `BergPDF-DEVELOPMENT-NOT-FOR-THE-STORE_…msix` and
Partner Center refuses it. The result is in `dist/packages/BergPDF_<version>.0_x64.msix`; the fourth version digit is
always 0 because the Store reserves it. **The package is deliberately unsigned:** upload it as it is and the Store
signs it. Raising the version needs nothing extra: it follows `Cargo.toml` (every submission must have a higher
version than the last).

## Trying the package before submitting (once, on Windows 11)

Option 1, no certificate: turn on *Settings ▸ System ▸ For developers ▸ Developer Mode*, then in PowerShell:

```powershell
Add-AppxPackage -Register dist\msix\layout\AppxManifest.xml
```

Option 2, the real package (as the Store would install it): sign a copy with a self-signed certificate whose subject
equals the package's `Publisher`, trust that certificate (administrator), install:

```powershell
$pub  = "CN=BergPDF Development"                       # the Publisher in the manifest
$cert = New-SelfSignedCertificate -Type Custom -Subject $pub -KeyUsage DigitalSignature `
          -FriendlyName "BergPDF test" -CertStoreLocation Cert:\CurrentUser\My `
          -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
$msix = (Get-ChildItem dist\packages\*.msix | Select-Object -First 1).FullName
& "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.0.26100.0\x64\signtool.exe" sign /fd SHA256 /a /sha1 $cert.Thumbprint $msix
Export-Certificate -Cert $cert -FilePath dist\bergpdf-test.cer
# administrator PowerShell:
Import-Certificate -FilePath dist\bergpdf-test.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople
Add-AppxPackage $msix
```

Remove the test copy and certificate afterwards (`Get-AppxPackage *BergPDF* | Remove-AppxPackage`, and delete the
certificate from *Trusted People*). The checklist in [`PLATFORM_CHECKLIST.md`](PLATFORM_CHECKLIST.md) ("Microsoft Store
package") lists what to look at. Also run the **Windows App Certification Kit** (part of the SDK) on the package: it is
the automated tests the Store runs too.

## Submission

*Product ▸ Start your submission*, then:

* **Pricing and availability:** free; markets as wanted.
* **Properties:** category *Productivity*; **privacy policy URL**
  `https://github.com/markvdberg0/BergPDF/blob/main/PRIVACY.md` (needed because the app can use the internet).
* **Age ratings:** answer the questionnaire (no user-generated content shared between users, no purchases, no
  location; the AI features send text to a provider chosen by the user).
* **Packages:** upload the `.msix`. Partner Center reads the manifest and shows the languages.
* **Store listing** (per language; Dutch and German match the interface languages): name, description, 1-3 screenshots
  (at least 1366×768; `docs/screenshots/` has candidates), the logo is taken from the package. Suggested description
  (English):

  > BergPDF is a fast, offline PDF editor. Edit existing text and images in place, add text in the fonts you have
  > installed, annotate, fill in and sign forms, reorganise pages, measure on drawings, search scanned pages with OCR,
  > and save as optimised or PDF/A files. Every change can be undone. Your documents stay on your computer: no
  > account, no telemetry. Optional extras (PDF Copilot questions and translation) only send text when you press a
  > button, to the AI provider you choose, with your own key. Free software under the GNU GPL v3.

* **Notes for certification** (field *Notes for certification*; this is what the reviewer reads):

  > BergPDF is a desktop PDF editor packaged as a full-trust Windows app. Open any PDF with File > Open, or via
  > "Open with". It needs the runFullTrust capability because it opens and saves user-chosen files anywhere on disk,
  > uses installed fonts and the GPU through DirectX 12, and registers as a PDF handler. Everything core works
  > offline. The internet is used only on a button press: PDF Copilot / Translate (needs the tester's own API key,
  > not required to review the app) and the optional download of OCR models (a verified data download from
  > ocrs-models.s3-accelerate.amazonaws.com; no executable code is downloaded). The Store copy has no update check of
  > its own. Source code: https://github.com/markvdberg0/BergPDF (GPL-3.0-or-later).

* **Restricted capability `runFullTrust`:** Partner Center asks for a justification; the text above is the
  justification. It is the normal declaration for a classic desktop program in an MSIX package.

## Things to know

* **Licence and terms.** BergPDF is GPL-3.0-or-later (D-001). As the copyright holder the owner may publish under the
  Store's terms, but every other contributor's code is under the GPL too. Keep the source for each released version
  available (the release tag), name the licence in the listing (the About dialog and `LICENSE` in the package do), and
  take legal advice if you are unsure how the Store's usage rules relate to the GPL for *users*.
* **Updates** reach users through the Store, usually within hours of certification. GitHub releases (NSIS installer) stay
  the route for people who do not use the Store; their copies keep the optional update check.
* **Settings and data folders.** Windows redirects the program's writes to `%APPDATA%` into the package's own storage,
  so a Store copy has its own preferences, recovery files and OCR models separate from a copy installed with the NSIS
  installer; uninstalling the Store copy removes them. Nothing is lost on update.
* **OCR models** are not bundled (D-023: redistribution terms unverified). They are downloaded on first use of OCR,
  into that storage. The Store's policy allows downloading data; it would not allow downloading code.
* **The program is not code-signed by us.** Inside the Store it is signed by Microsoft, so there is no SmartScreen
  warning for Store installs.
* **Windows on ARM** runs the x64 package under emulation. A native ARM64 package would need a second build target.
