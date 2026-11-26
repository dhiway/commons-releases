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

The key is created once on the owner's machine, never on a builder:

```sh
ssh-keygen -t ed25519 -N '' -C "commons-releases beta-1 (CI)" -f beta-1
```

`beta-1.pub` goes to `keys/beta-1.pub`. The contents of `beta-1` go into a
GitHub Actions secret, `RELEASES_BETA_SIGNING_KEY`, in an environment named
`release-beta` that requires the owner's approval. Then delete `beta-1` from
the owner's machine. `dev` is the same with `dev-1`.

The signing job runs in that environment on a GitHub-hosted runner, never on
an AWS builder:

```sh
umask 077
printf '%s\n' "$RELEASES_BETA_SIGNING_KEY" > "$RUNNER_TEMP/beta-key"
scripts/sign-manifest.sh beta "$RUNNER_TEMP/beta-key"
rm -f "$RUNNER_TEMP/beta-key"
```

No workflow file exists yet. The job needs the environment and the secret
first, which the owner creates on the GitHub repository.

## The owner: signing `dev` until the CI job exists

Until then, the owner signs `dev` on their own machine with a `dev-1` key file
that stays there, protected by a passphrase. Create it once:

```sh
ssh-keygen -t ed25519 -C "commons-releases dev-1" -f ~/.ssh/commons-releases-dev-1
cp ~/.ssh/commons-releases-dev-1.pub keys/dev-1.pub    # commit this file
```

Then follow steps 1, 2 and 4 of signing `stable`, with `dev/manifest.json`.
Step 3 becomes:

```sh
scripts/sign-manifest.sh dev ~/.ssh/commons-releases-dev-1 <assets-dir>
```

That runs
`ssh-keygen -Y sign -f ~/.ssh/commons-releases-dev-1 -n commons-release-manifest dev/manifest.json`,
which asks for the key's passphrase and writes `dev/manifest.json.sig`. The
script then verifies the result against `keys/dev-*.pub` and the assets. Any
`ssh-keygen` will do for a key file: Apple's lacks only FIDO support. When the
CI job takes over, move the key into the `release-dev` environment's secret
as described above, and delete it from the machine.

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
