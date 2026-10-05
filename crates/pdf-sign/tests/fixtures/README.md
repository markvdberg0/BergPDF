# Signing test fixtures

**Test-only credentials. The private keys are public on purpose. Never use them for anything real.**

All PKCS#12 files use the password `test123`. Generated with OpenSSL 3.0.13:

* `rsa-aes.p12` — self-signed RSA-2048, modern PBES2/AES encryption
* `rsa-legacy3des.p12` — same key, legacy PBE-SHA1-3DES encryption (older exporters)
* `ec-aes.p12` — self-signed ECDSA P-256
* `chain-aes.p12` — leaf certificate issued by `ca.crt` with the CA included in the file

The `.crt` files are the public certificates (used for independent checks with `openssl`). The PEM private keys were deleted so no plain-text private key sits in the repository; the keys live only inside the password-protected `.p12` files (extract with `openssl pkcs12 -nocerts -nodes` if ever needed).
