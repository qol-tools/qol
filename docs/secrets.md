# Secrets

qol reads a secret, such as an agent account's token, by name from the credential store of the computer it runs on.
Only the name travels with the qol profile; each computer stores the value once, in its own credential store.

## Storing a secret

```sh
qol secrets store <name>
```

With a terminal, it prompts for the secret without echoing it.
Without one, it reads the secret from stdin, so a password manager's command can pipe it in.
The secret is never a command-line argument, so it never shows in the process list.
Storing under a name that already has a secret replaces it.
`qol secrets store help` lists what it accepts and refuses.

## Where each system keeps it

| System | Store | Entry | Written by |
| --- | --- | --- | --- |
| Linux | Secret Service (GNOME Keyring, KWallet) | label `<name>`, attribute `service=<name>` | `secret-tool store`, from libsecret-tools |
| macOS | login Keychain | generic password, service `<name>`, account `$USER` | `/usr/bin/security -i`, the command on its stdin with the secret as hex (`-X`), the name in escaped double quotes |
| Windows | Credential Manager | generic credential, target `<name>`, kept for this computer, value in UTF-16 | `CredWriteW` |

On Linux and macOS this is the same entry these commands make by hand, so a secret stored either way is the one qol reads:

```sh
secret-tool store --label=<name> service <name>
security add-generic-password -U -a "$USER" -s <name> -w
```

After storing, qol reads the secret back by name and fails when it reads something else.
That happens when another entry with the same name comes first, such as a Keychain entry for the same service under another account.

## Reading a secret

Code reads a secret with `qol_secrets::read(<name>)`.
`claude_accounts` in `sessions.toml` names the secret a Claude config dir's lanes run under; see `qol sessions lane-exec help`.
