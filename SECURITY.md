# Security

BergPDF opens files that may come from anyone, so reports about crashes, memory use or unsafe behaviour on a
hostile PDF are welcome.

**Report privately** with GitHub's *Security → Report a vulnerability* on this repository rather than in a public
issue, and include a file that reproduces it if you can. Please give us reasonable time to fix it before you
publish details. There is no bug bounty.

Supported: the latest release.

What BergPDF does and does not isolate (for example that rendering runs in the same process, and that the
optional AI API key is kept in a plain file readable only by your account) is described in
[`docs/SECURITY.md`](docs/SECURITY.md). The private keys under `crates/pdf-sign/tests/fixtures` are throw-away
test keys published on purpose.
