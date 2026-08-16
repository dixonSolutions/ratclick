# Repository signing key

`ratclick.asc` is the public half of the key that signs the apt and dnf
repositories published to GitHub Pages. It is committed here so the key you
download from the site can be checked against the one in version control.

```
pub   rsa4096 2026-08-16 [SC]
      9FCF 7330 2A38 BB3D E863  79E3 0C2E A709 64F7 D273
uid   RatClick Repository Signing Key (https://github.com/dixonSolutions/ratclick)
```

Verify what the site serves matches this file:

```bash
curl -fsSL https://dixonsolutions.github.io/ratclick/ratclick.asc \
  | gpg --show-keys --with-fingerprint
```

The private half lives only in the repository's GitHub Actions secrets
(`GPG_PRIVATE_KEY`, `GPG_PASSPHRASE`) and is used solely by the release
workflow. It is not a personal identity key: if it is ever compromised,
generate a new one, replace both secrets, and re-run the workflow — every
consumer then has to re-import the new key.
