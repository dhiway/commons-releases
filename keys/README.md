# Public keys

One OpenSSH public key per file, `<channel>-<n>.pub`, numbered in the order the
keys are created. A channel has two slots, the key in use and the next key.
[docs/signing.md](../docs/signing.md) explains how the keys are made, who holds
them and how a rotation works.

`dev-1.pub` and `beta-1.pub` are the CI keys. Their private halves exist only
as the GitHub Actions secrets `RELEASES_DEV_SIGNING_KEY` and
`RELEASES_BETA_SIGNING_KEY`, in the `release-dev` and `release-beta`
environments, which require the owner's approval before a job may use them.
They were generated once and deleted from the owner's machine.

There is no `stable` key yet. The owner creates `stable-1` and `stable-2` on
hardware tokens.

Nothing in this repository's tests uses a key from here: each test generates
its own throwaway key.
