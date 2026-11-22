# INT-1497: three-platform staging packages

## Intent

Build installable packages for human email-account acceptance against staging, without making a
release or treating a package build as acceptance. Target Linux x86_64, macOS universal and
Windows x64 only. Publish no release, tag or update feed; do not use production configuration,
signing credentials or notarization. Existing macOS ad-hoc packaging remains unchanged.

## Approaches

- **Selected:** A separate, manually dispatched staging entry calls the existing nonpublishing
  package workflow. This makes staging runs distinguishable without duplicating installers.
- Adding a switch to the normal entry is smaller but makes staging and ordinary runs easier to
  confuse. Duplicating the workflows separates entry points but risks inconsistent packaging.

## Design

1. A job gated by GitHub's `staging` environment snapshots `.github/config/intcad-staging-account.json`
   from a clean checkout at the selected run SHA. The file contains only these exact public keys:
   `INTCAD_WORKOS_PUBLIC_CLIENT_ID`, `INTCAD_AUTHKIT_VERIFICATION_ORIGIN` and
   `INTCAD_PRODUCT_HTTPS_ORIGIN`. Missing or malformed values fail before packaging. No secret,
   runtime GitHub variable (of any scope), or production default may substitute for this file.
   The initial values are the exact approved public staging tuple. Any rotation requires review of
   the new tuple and SHA. The environment remains an approval gate, not a data source.
2. Use the same snapshot for Linux and Windows packaging and **both** macOS compile jobs. The
   universal macOS join consumes existing binaries and does not compile or read mutable staging
   inputs. Ordinary package runs keep their existing targets, names, evidence, and account
   configuration behavior.
3. Staging Actions artifact names distinguish staging from ordinary packages. Evidence includes
   the fixed source SHA, target, artifact SHA-256s, and the public tuple, checked-in path and SHA
   for that same snapshot. The reusable workflow verifies the snapshot against the fixed checkout
   before dependent runners and in the evidence step. Do not claim a login, keychain, or
   installed-app acceptance pass from metadata.

## Approval boundary

Owner approved local implementation and review. A reviewed fixed commit, checked staging
configuration, and explicit authorization are required before any push or CI dispatch.
This design does not enable controlled execution or authorize production changes.
