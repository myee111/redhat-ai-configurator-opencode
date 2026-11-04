# Red Hat AI Configurator for OpenCode

A terminal installer that installs and configures the
[OpenCode](https://opencode.ai) AI coding agent for two model services: a
ratatui TUI for people, and `--non-interactive` for Ansible and image builds. It
validates each key before writing anything, stores it through
`opencode auth import`, backs up every file it replaces, and verifies the result
with `opencode models`.

| Provider | Internal name | OpenCode provider ID | Runtime package |
|---|---|---|---|
| Red Hat AI MaaS | `rhai-maas` | `ai-bu` | OpenAI compatible |
| EnMaaS | `enmaas` | `enmaas` | Anthropic compatible |
| EnMaaS (Gemini) | `enmaas` | `enmaas-oai` | OpenAI compatible |

Red Hat AI MaaS needs the corporate VPN **off**; EnMaaS needs it **on**. The
installer shows the required state at each step.

Service endpoints and API keys are **not embedded**. Endpoints come from flags,
the environment, or `/etc/redhat-ai-configurator-opencode/endpoints.conf`; keys
come from the environment or a key file. See [Reference](#reference).

## Install

RHEL 10.2 (x86_64 or aarch64):

```sh
sudo dnf install ./redhat-ai-configurator-opencode-0.1.0-1.el10.x86_64.rpm
```

The RPM does not require OpenCode. On first run the installer downloads the
current Linux release to `~/.local/bin` when `opencode` is not on `PATH`, so no
subscription repository or package is needed. Pass `--install-dir /usr/local/bin`
to install elsewhere (you will need write access), or `--no-install` to manage
the CLI yourself.

Run the installer **as yourself**, not as root:

```sh
redhat-ai-configurator-opencode
```

## Interactive use

The TUI walks through five screens:

1. **System** — OS, user, configuration directory, OpenCode version, VPN state,
   endpoint warning, and service reachability.
2. **Providers** — space to toggle, enter to continue.
3. **Keys** — the key page, the VPN requirement, and a masked input. Enter
   validates; if a key is already stored, enter keeps it and `ctrl+u` clears it.
4. **Defaults** — default model per provider and the default agent (`build` or
   `plan`).
5. **Review** — the files that will be written and the keys that will be
   imported. Enter applies.

`esc` goes back, `ctrl+c` quits, and the recap stays in scrollback.

## Non-interactive use

```sh
export RHAI_MAAS_BASE_URL=https://maas.internal/v1
export ENMAAS_BASE_URL=https://enmaas.internal
export RHAI_MAAS_API_KEY=...
export ENMAAS_API_KEY=...

redhat-ai-configurator-opencode --non-interactive \
  --provider rhai-maas,enmaas \
  --format json
```

### Keys

The environment is read first:

```sh
export RHAI_MAAS_API_KEY=...
export ENMAAS_API_KEY=...
```

Or a file, which must be mode 0600 and owned by you:

```sh
# keys.env, mode 0600, owned by you
RHAI_MAAS_API_KEY=...
ENMAAS_API_KEY=...

redhat-ai-configurator-opencode --non-interactive \
  --provider rhai-maas,enmaas --key-file keys.env
```

Secret managers can feed the key through stdin:

```sh
# or from stdin, for a secret manager
pass show work/rhai-maas | redhat-ai-configurator-opencode --non-interactive \
  --provider rhai-maas --key-file - --no-verify
```

Keys are imported with `opencode auth import` and stored in OpenCode's own
credential database. The tool never edits that database directly and never
prints a key back.

### Common flags

| Flag | Purpose |
|---|---|
| `--provider rhai-maas,enmaas` | Providers to configure (required in non-interactive mode) |
| `--rhai-maas-base-url` / `--enmaas-base-url` | Override the endpoint for this run |
| `--key-file PATH` / `-` | Read keys from a 0600 file or stdin |
| `--keep-existing` | Keep a key already stored in OpenCode |
| `--model enmaas=rits/zai-org/glm-5-3` | Choose the default model |
| `--agent build` / `--agent plan` | Choose the default primary agent |
| `--install-dir DIR` | Where a downloaded OpenCode binary is placed |
| `--no-install` | Never download OpenCode |
| `--no-validate` | Skip the key check (offline builds) |
| `--require-validation` | Fail on an unverified key or placeholder endpoint |
| `--no-verify` | Skip `opencode models` |
| `--dry-run` | Print the plan and write nothing |
| `--format json` | Machine-readable report |

### Ansible

```yaml
- name: Configure OpenCode for the AI MaaS providers
  hosts: workstation
  become: false
  tasks:
    - name: Configure OpenCode
      ansible.builtin.command:
        argv:
          - redhat-ai-configurator-opencode
          - --non-interactive
          - --provider
          - rhai-maas,enmaas
          - --key-file
          - /run/secrets/ai-keys.env
          - --format
          - json
      register: configured
      changed_when: (configured.stdout | from_json).changed
```

The JSON report has `ok`, `changed`, `changedFiles`, `binary`, `providers`,
`agent`, and `warnings`.

## Reference

### Endpoints

```sh
# /etc/redhat-ai-configurator-opencode/endpoints.conf
RHAI_MAAS_BASE_URL=https://maas.internal/v1
ENMAAS_BASE_URL=https://enmaas.internal
```

`RHAI_MAAS_BASE_URL` is the OpenAI-compatible root and **must include `/v1`**.
`ENMAAS_BASE_URL` is the Anthropic-compatible root and **must not include
`/v1`**; the installer appends it. Flags win over the environment, which wins
over the file. A fresh install uses `example.com` placeholders and warns until
they are replaced.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Success, or a successful dry run |
| `1` | Unexpected error |
| `2` | Usage error, placeholder endpoints with `--require-validation`, no provider, or no TTY without `--non-interactive` |
| `3` | OpenCode is missing, older than 2.0, or could not be downloaded |
| `4` | A key is missing, unreadable, or was rejected |
| `5` | A service is unreachable (network or VPN) |
| `6` | A configuration file, credential import, or download could not be written |
| `7` | Written, but `opencode models` did not list the configured models |

### What gets written

`~/.config/opencode/opencode.json` is merged, not replaced:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "model": "enmaas/rits/deepseek-ai/DeepSeek-V4.1-Flash-1017",
  "default_agent": "build",
  "providers": {
    "ai-bu": {
      "name": "Red Hat AI MaaS",
      "package": "@opencode/ai/providers/openai-compatible",
      "env": ["RHAI_MAAS_API_KEY"],
      "settings": { "baseURL": "https://maas.internal/v1" },
      "models": {
        "publishers/prelude-maas/models/glm-53-flash": {
          "name": "GLM 5.3 Flash",
          "limit": { "context": 200000, "output": 65536 }
        }
      }
    }
  }
}
```

Keys, providers and models the installer does not own are preserved; it only
replaces its own provider entries plus `model` and `default_agent`. JSONC input
is accepted, though the rewrite is plain JSON. Every replaced file is backed up to `<file>.bak-<timestamp>`.
If both services are configured, EnMaaS wins so **DeepSeek 4.1 Flash** is the
default; with only Red Hat AI MaaS, **GLM 5.3 Flash** is.

### Security

- The API key never appears in output, logs, or the JSON report.
- `--key-file` is rejected unless it is 0600 and owned by you.
- Keys travel to `opencode auth import` through a 0600 temporary file that is
  removed immediately, even on failure.
- The tool never edits OpenCode's SQLite database, and never reads a key back
  out to display it.

### Build

```sh
cargo test --locked
cargo build --release --locked
```

#### RPM

```sh
make rpm        # writes dist/rpmbuild/RPMS/.../redhat-ai-configurator-opencode-*.rpm
```

### Layout

| Path | Purpose |
|---|---|
| `src/catalog.rs` | Providers, models, OpenCode package and base-URL shape |
| `src/opencode/` | Config merge, provider entries, credential import, CLI install and verification |
| `src/cli.rs` | Flags, non-interactive mode, doctor |
| `src/tui.rs` | Interactive installer |
| `src/settings.rs` | Endpoint resolution and warnings |
| `packaging/` | RPM spec, man page, endpoints example |

## License

MIT
