# Public keys

One OpenSSH public key per file, `<channel>-<n>.pub`, numbered in the order the
keys are created. A channel has two slots, the key in use and the next key.
[docs/signing.md](../docs/signing.md) explains how the keys are made, who holds
them and how a rotation works.

None exist yet. The owner creates `stable-1` and `stable-2` on hardware tokens.
The `dev` and `beta` keys are made for CI. Nothing in this repository's tests
uses a key from here: each test generates its own throwaway key.
