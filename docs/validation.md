# Validation on RHEL 10

The installer was exercised on a throwaway GCP instance on 2026-11-04 and then
deleted. This is the record, so a later change can be checked against it.

## Environment

| | |
|---|---|
| Image | RHEL 10.2 lab image, x86_64 |
| OS | RHEL 10.2 (Coughlan), glibc 2.39, x86_64 |
| Machine | `e2-medium`, GCP, default network, no service account |
| Binary | release build cross-compiled with `cargo zigbuild --target x86_64-unknown-linux-gnu.2.39` |

The instance was labelled `managed-by=redhat-ai-configurator-opencode` so it
could be found and deleted in the same session.

## What was checked

1. **Cold install.** With no `opencode` on PATH, the tool downloaded
   `opencode-linux-x64.tar.gz`, installed 2.2.5 to `~/.local/bin/opencode`, and
   reported the path and version.
2. **Configuration.** `--non-interactive --provider rhai-maas,enmaas
   --no-validate` wrote `~/.config/opencode/opencode.json` with `ai-bu`,
   `enmaas` and `enmaas-oai`, `model` set to
   `enmaas/rits/deepseek-ai/DeepSeek-V4.1-Flash-1017`, `default_agent: build`,
   and 6/10/4 models respectively.
3. **Credentials.** `opencode auth list` showed
   `Red Hat AI Configurator | stored` for both EnMaaS and Red Hat AI MaaS.
4. **Verification.** `opencode models` listed every configured model, including
   `enmaas/rits/deepseek-ai/DeepSeek-V4.1-Flash-1017`; the report returned
   `"verified": true`.
5. **Idempotence.** A second identical run returned `changed: false`,
   `authUpdated: false`, `backup: null`.
6. **Keep existing.** A run with `--keep-existing` and no keys in the
   environment succeeded without writing anything.
7. **Doctor.** `--doctor` passed user, binary, config, model, agent, providers,
   endpoints and both stored keys; the two reachability checks failed as
   expected against `.invalid` endpoints.
8. **Key validation path.** `--require-validation` against an unreachable
   endpoint exited `5` with the VPN hint.
9. **TUI.** Started over SSH, rendered the five-step header, provider list and
   connectivity card at 120x40, accepted `q` and exited with
   "Cancelled; nothing was written."

The TUI is not usable without a window size: over a non-tty `ssh -tt` the PTY
reports 0x0 and the alternate screen stays blank. Run it from a real terminal
(or set `stty rows/cols` first).

## Reproduce

```sh
gcloud compute instances create rhai-opencode-test \
  --project PROJECT --zone ZONE --machine-type e2-medium \
  --image-family RHEL10_FAMILY --image-project PROJECT \
  --network default --no-service-account --no-scopes

cargo zigbuild --release --locked --target x86_64-unknown-linux-gnu.2.39
gcloud compute scp target/x86_64-unknown-linux-gnu/release/redhat-ai-configurator-opencode \
  rhai-opencode-test:/tmp/rhai-opencode --zone ZONE

gcloud compute ssh rhai-opencode-test --zone ZONE --command '
  export RHAI_MAAS_BASE_URL=https://maas.invalid/v1 ENMAAS_BASE_URL=https://enmaas.invalid
  export RHAI_MAAS_API_KEY=sk-test ENMAAS_API_KEY=sk-test
  chmod +x /tmp/rhai-opencode
  /tmp/rhai-opencode --non-interactive --provider rhai-maas,enmaas --no-validate --format json'
```

The test used dummy keys and `.invalid` endpoints, so no real service was
contacted. `--require-validation` with real endpoints and keys is the check to
run before a release.
