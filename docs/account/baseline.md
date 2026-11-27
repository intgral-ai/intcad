# INT-1497 client baseline

- Implementation base: `9714e5b300030f23bc775fa180b1ba7c80047f80`.
- `git ls-remote --exit-code origin refs/heads/develop` and local `origin/develop` both returned that commit.
- The new isolated worktree started clean. The original `sync/upstream-v0.41.0` checkout at `202f8acda36d1009f7dc8ced69ff27479b63da53` had 599 tracked/index changes and was left untouched.
- `git ls-tree` confirmed the existing Review panel source blob `ce00861c3e8228daff745906a0e05c9057c92ad5`.

The [frozen shared contract](https://linear.app/intgral/document/3c7c6dfefe52) is `intcad-auth/1.0`, rev 3. `sha256sum` on the extracted JSON contents matched all three pins:

| Artifact | SHA-256 |
| --- | --- |
| OpenAPI | `20f32b8cde8141220489cbd0db4dafd42a01a1d14d9ad9c20078807c5c8afcb8` |
| Fixtures | `ee1d8dde51fac40debf95d1bc29555b787adfa60175901f98be2b97c73529b2b` |
| Manifest | `f4d677dd066429854eac45e7776f7e2611845db9c174e26833704af5faf6b45e` |

This is static preflight evidence. It does not certify client behavior, server integration, or any platform package.
