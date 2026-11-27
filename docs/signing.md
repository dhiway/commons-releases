# Signing a manifest

Each channel's `manifest.json` carries a detached signature,
`manifest.json.sig`, made with OpenSSH's `ssh-keygen -Y sign` and an Ed25519
key. Only the manifest is signed. It lists every asset's SHA-256 and size, so
the assets and `SHA256SUMS` are covered through it.

## What the signature covers

- **The bytes**: the manifest file exactly as published, with no
  re-encoding ([manifest.md](manifest.md#what-is-signed)).
- **The structure**: OpenSSH's SSHSIG format (`PROTOCOL.sshsig` in the
  OpenSSH sources). The key signs a structure that holds the namespace
  `commons-release-manifest` and the SHA-512 of the file. Because of the
  namespace, a signature the same key made for anything else (a Git commit,
  another file) does not verify as a manifest signature.
- **On a hardware key** (`sk-ssh-ed25519@openssh.com`): the token signs
  SHA-256 of its application string, a flags byte, a counter and SHA-256 of
  that structure (`PROTOCOL.u2f`). The verifier requires the flag that says
  the token was touched.

## Why `ssh-keygen` and not minisign

The owner's ruling puts the `stable` key on a hardware token. minisign keeps
its secret key in a password-protected file and cannot use a token. OpenSSH
can: an `ed25519-sk` key lives on any FIDO2 token that supports EdDSA (a
YubiKey 5 with firmware 5.2.3 or later, for example), and `ssh-keygen -Y sign`
signs with it. The same command signs with an ordinary key file in CI, so the
owner and CI use one tool and one signature format, and anyone can check a
signature with `ssh-keygen -Y verify` as well as with
`commons-release-verify`.

Apple's `/usr/bin/ssh-keygen` has no FIDO support ("No FIDO
SecurityKeyProvider specified"). Use Homebrew's: `brew install openssh`, then
`/opt/homebrew/bin/ssh-keygen`.

## Where the public keys live

`keys/<channel>-<n>.pub`, one OpenSSH public key line per file, numbered in
the order keys are created. Each client compiles in two slots per channel:
the key in use and the next key. The files here are the reference copy;
what a client trusts is what it was built with.

| Channel | Key | Held by |
|---|---|---|
| `stable` | `ed25519-sk` on a hardware token | the owner |
| `beta` | `ed25519` key file | a CI secret in a protected environment |
| `dev` | `ed25519` key file | a CI secret in a protected environment |

Each channel has its own keys, so the `dev` key cannot sign `beta` and a CI
key can never sign `stable`.

## The owner: creating the `stable` keys (once)

Enrol two tokens: one for daily use, one kept offline as the next key. Then a
lost or broken token does not strand clients, because they already trust the
second.

```sh
/opt/homebrew/bin/ssh-keygen -t ed25519-sk -O resident -O verify-required \
  -O application=ssh:commons-releases-stable-1 \
  -C "commons-releases stable-1" -f ~/.ssh/commons-releases-stable-1
cp ~/.ssh/commons-releases-stable-1.pub keys/stable-1.pub
# Same again with the second token: stable-2, then store that token offline.
```

- `-O resident` keeps the key on the token, so `ssh-keygen -K` can recover
  the handle file on another machine.
- `-O verify-required` asks for the token's PIN at every signature.
- The file `~/.ssh/commons-releases-stable-1` is a handle, not the key. It
  signs nothing without the token.

Commit `keys/stable-1.pub` and `keys/stable-2.pub`.

## The owner: signing `stable`

1. Fetch the builders' output for the release: the archives, `SHA256SUMS` and
   `assets.json` ([building.md](building.md)). Check it:
   `shasum -a 256 -c SHA256SUMS`.
2. Edit `stable/manifest.json`: add or update the set from `assets.json`,
   raise `sequence` by one, set `published` to now and `expires` (suggested:
   90 days later).
3. Sign and verify:

   ```sh
   SSH_KEYGEN=/opt/homebrew/bin/ssh-keygen \
     scripts/sign-manifest.sh stable ~/.ssh/commons-releases-stable-1 <assets-dir>
   ```

   It first compares `sequence` with the manifest committed at `HEAD`, the
   last one published, and stops before signing if it was not raised, so no
   touch is wasted. Then it runs
   `/opt/homebrew/bin/ssh-keygen -Y sign -f ~/.ssh/commons-releases-stable-1 -n commons-release-manifest stable/manifest.json`,
   which asks for the PIN and a touch and writes `stable/manifest.json.sig`.
   It then runs `commons-release-verify` with `keys/stable-*.pub` (see
   [Rotating a key](#rotating-a-key) for three key files), with
   `--last-sequence` one above the committed sequence, checking the assets in
   `<assets-dir>` as well. If the verification fails, it removes the
   signature. The key and the assets directory may be given relative to the
   directory you run it from.
4. Commit `stable/manifest.json` and `stable/manifest.json.sig` together, and
   do not touch the manifest again.

Re-sign before `expires` even when nothing changed: raise `sequence`, update
`published` and `expires`, and repeat step 3.

## CI: signing `dev` and `beta`

CI signs `dev` and `beta` with `.github/workflows/sign-manifest.yml`, on a
GitHub-hosted runner, never on an AWS builder. `stable` is not in that
workflow: it is signed only on the owner's hardware token.

The keys were generated once, on the owner's machine and never on a builder:

```sh
umask 077
/opt/homebrew/bin/ssh-keygen -t ed25519 -N '' -C "commons-releases dev-1 (CI)"  -f dev-1
/opt/homebrew/bin/ssh-keygen -t ed25519 -N '' -C "commons-releases beta-1 (CI)" -f beta-1
```

The public halves are committed at `keys/dev-1.pub` and `keys/beta-1.pub`. The
private halves exist only as the GitHub Actions secrets
`RELEASES_DEV_SIGNING_KEY` and `RELEASES_BETA_SIGNING_KEY`, in the
environments `release-dev` and `release-beta`, which require the owner's
approval before a job may use them. The private key files were then deleted
from the owner's machine; a private key exists in no checkout. If a secret is
lost, generate the next key as a planned rotation (below); never weaken the
check to recover from it.

### Running the signing job

`scripts/sign-manifest.sh` refuses a working-tree sequence that is not higher
than the one committed at `HEAD`, so a manifest cannot be signed once it is on
`main`. The workflow therefore signs from a pull request:

1. Open a branch that adds or raises exactly one file,
   `<channel>/manifest.json`, and nothing else. The job refuses any other
   change.
2. Open a pull request for that branch against `main`.
3. Run **Actions → sign-manifest → Run workflow** on `main` with the channel
   (`dev` or `beta`), the pull request number, and the head commit SHA of the
   branch you reviewed, then approve the environment when GitHub asks. The
   job runs only from `main`: the environments name `main` as the only branch
   a deployment may come from, and the workflow itself refuses any other ref.
   The head commit is what binds the approval to the content: the job fetches
   the pull request, refuses unless it is at exactly that commit, and prints
   the manifest's SHA-256 in the log before signing it.
4. The job checks out `main`, applies only that manifest to the working tree,
   signs it with the channel's key from the environment secret, and pushes
   `<channel>/manifest.json.sig` as a commit on the pull request's branch.
5. The pull request now holds the manifest and its signature together; merge
   it. The job never writes to `main`.

The job refuses a channel other than `dev` or `beta`, a pull request from a
fork or one that does not target `main`, a pull request that changes any path
but the one manifest (a rename counts as two paths), a manifest that is not a
regular file (a symlink), a branch name it cannot use as a ref, and a pull
request whose head is not the commit given at dispatch. The token it fetches
and pushes with is used for those commands only; it is never written to the
checkout's Git configuration. It takes the signing script and the public keys
it verifies against from `main`, not from the pull request, so the code that
runs beside the key cannot be changed by the change being signed. The script
verifies the signature against `keys/<channel>-*.pub` before the job reports
success.

The manifest's `published` must not be more than an hour ahead of the runner's
clock, and `expires` must still be in the future, because the verifier checks
both.

Any `ssh-keygen` will do for a key file: Apple's lacks only FIDO support. The
job does not need FIDO because `dev` and `beta` are ordinary key files.

## Rotating a key

The two slots are the key in use (`current`) and the next key (`next`).
Clients built today carry both.

- **Planned rotation.** Start signing with the next key: the verifier accepts
  it from the second slot. In the next client release, the next key becomes
  `current`, and a newly created key (`<channel>-3`) becomes `next`. Once no
  supported client carries the old key, delete its `.pub` file from `keys/`.

  Until then `keys/` holds three files, and `sign-manifest.sh` accepts that.
  It reads the files as numbered keys and treats each two consecutive
  numbers as the pair one generation of clients carries: with keys 1, 2 and
  3, older clients carry 1 and 2, newer ones 2 and 3. It verifies the new
  signature once per pair, so it accepts a signature only if every client
  still served would. With three files that is key 2 alone; signing with key
  3 too early, or with key 1 too late, is refused and the signature removed.
  A file in `keys/` not named `<channel>-<number>.pub` is refused.
- **A lost or compromised key.** Sign with the next key at once, and release
  clients that no longer trust the old one. A client that still trusts a
  compromised key stays at risk until it updates: neither `expires` nor
  `sequence` helps, because the holder of the key can sign a fresh manifest
  with a higher sequence. That is why the next key is created in advance and
  kept offline.
