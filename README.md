# civitfit

Right-size Civitai image models to your hardware.

`civitfit` reads your machine's GPU memory budget, looks at image checkpoints on
[Civitai](https://civitai.com) and [civitai.red](https://civitai.red), and prints
what will actually run — with a peak-memory estimate and a rough generation
time. It is the image-model equivalent of [llmfit](https://github.com/AlexsJones/llmfit):
detect hardware, score candidates, show a table.

Image models only. Video models are filtered out.

## Install

```sh
cargo install --path .

# or run the release build directly
cargo build --release
./target/release/civitfit system
```

The binary has no runtime dependencies and no Python requirement.

## Quick start

```sh
civitfit system                       # what did it detect?
civitfit fit                          # top 20 checkpoints for this machine
civitfit fit -n 40                    # more rows
civitfit fit --base "SDXL 1.0"        # narrow by Civitai base model
civitfit fit --base "SDXL 1.0,Illustrious"
civitfit fit --sort speed             # fastest first
civitfit search "z image"             # search by name
civitfit info 277058                  # one model by id
civitfit info https://civitai.red/models/277058
civitfit --source both                # query civitai.com and civitai.red
civitfit --json fit -n 5              # machine-readable output
civitfit arch                         # the architecture registry
```

Global options such as `--json`, `--gpu`, `--source`, `--width` and `--steps`
work before or after the subcommand.

```text
=== Civitai models that fit ===  (97 scored, showing 6, 3 filtered)
  source civitai.com  ·  sort best  ·  1024x1024

╭─────────────┬───────────────┬────────────────┬──────────┬─────────┬───────┬────────┬───────────╮
│ Status      │ Model         │ Arch           │ Size     │ Peak    │ Mem   │ Time   │ Downloads │
├─────────────┼───────────────┼────────────────┼──────────┼─────────┼───────┼────────┼───────────┤
│ 🟡 Tight    │ FLUX          │ FLUX.1         │ 22.17GiB │ 24.4GiB │ 83%   │ 84s    │ 355k      │
│ 🟢 Perfect  │ Anima         │ Anima          │ 3.90GiB  │ 6.1GiB  │ 21%   │ 45s    │ 287k      │
│ 🟢 Perfect  │ Z Image Turbo │ Z-Image        │ 11.46GiB │ 13.7GiB │ 47%   │ 26s    │ 99k       │
╰─────────────┴───────────────┴────────────────┴──────────┴─────────┴───────┴────────┴───────────╯
```

## Commands

| Command | What it does |
| --- | --- |
| `system` | Show detected OS, chip, RAM, GPU budget and backend. |
| `fit` | Browse checkpoints that fit this machine. |
| `search <query>` | Search Civitai by name, then score the results. |
| `info <id\|url>` | Full per-version breakdown for one model. |
| `plan <id\|url>` | Alias for `info`. |
| `arch` | List the embedded architecture registry. |

Useful flags:

| Flag | Meaning |
| --- | --- |
| `-n, --top N` | Rows to display (default 20 for `fit`, 10 for `search`). |
| `--limit N` | Models to fetch per page (max 100). |
| `--pages N` | Pages to fetch for `fit` (max 10). |
| `--base NAME` | Civitai base-model filter; comma-separated values allowed. |
| `--type TYPE` | Civitai model type (default `Checkpoint`; full checkpoints only). |
| `--sort MODE` | `best`, `downloads`, `newest`, `speed` or `memory`. |
| `--source NAME` | `civitai.com`, `civitai.red` or `both`. |
| `--nsfw` | Include NSFW results (off by default). |
| `--include-other` | Keep models whose base model is `Other`/`Unknown`. |
| `--gpu SIZE` | Override the GPU budget, e.g. `--gpu 24G`. |
| `--ram SIZE` | Override RAM, e.g. `--ram 64G`. |
| `--width/--height` | Target resolution for memory and speed (default 1024×1024). |
| `--steps N` | Override sampler steps for the speed estimate. |
| `--all` | In `info`, show every model version, not just the latest 3. |
| `--json` | Machine-readable output. |

## How the verdict is calculated

### GPU budget

- **macOS:** `sysctl iogpu.wired_limit_mb` when set, otherwise 85% of RAM.
- **Linux:** `nvidia-smi` total VRAM, otherwise 50% of RAM.
- **Fallback:** 50% of RAM or an explicit `--gpu`.

All sizes are GiB.

### Peak memory

```text
peak = max(weights, heaviest_companion × 0.55) + activations + 0.5 GiB
```

- `weights` is the checkpoint file size reported by Civitai (`sizeKB / 1048576`).
- Text encoders and VAEs are usually released before the sampling loop, so the
  heaviest companion is weighted at 55% instead of being counted in full.
- Bundled checkpoints (SDXL, SD 1.5, Pony, Illustrious, …) skip companions.
  Large non-bundled files are treated as bundled when
  `heaviest_file / (params_b × 2) >= 1.25`.
- Activations are 1.7 GiB up to 1.5 MP, 2.6 GiB up to 3 MP, 4.0 GiB up to
  6 MP, and 6.0 GiB above that.
- The flat 0.5 GiB covers the runtime, samplers and fragmentation.

### Status

| Status | Rule |
| --- | --- |
| 🟢 Perfect | peak ≤ 60% of the budget |
| 🟢 Good | peak ≤ 75% |
| 🟡 Tight | peak ≤ 90% |
| 🟠 Marginal | peak ≤ 100% |
| 🔴 Won't run | peak > 100% |
| ⛔ Unsupported | no loadable weights, or ComfyUI support is known to be absent |

### Speed

```text
seconds = k_sec_per_step × steps × megapixels
```

`k_sec_per_step` comes from the architecture registry. Only Z-Image has been
measured on real hardware so far (8 steps at 1024×1024 ≈ 26s on Apple Silicon).
Treat other numbers as rough guidance, not benchmarks.

## Default ordering

`--sort best` puts runnable models first, then orders by architecture tier
(frontier → modern → SDXL-class → legacy), then by a popularity score built from
downloads and likes. This keeps a sea of old SD 1.5 checkpoints from burying
newer SDXL, FLUX, Z-Image, Anima and Qwen models.

Use `--sort downloads` for pure popularity, `--sort newest` for recent uploads,
`--sort speed` for the fastest, or `--sort memory` for the smallest footprint.

## What is filtered out

- Video base models (Hunyuan Video, Wan, LTX-Video, CogVideoX, Mochi, …).
- Models whose Civitai base model is `Other` or `Unknown` (disable with
  `--include-other`).
- Versions with no loadable weights, such as ONNX-only helpers or training-data
  ZIPs. They are shown as Unsupported in `info`.
- NSFW results unless `--nsfw` is passed.

`civitfit` does not provide NSFW or uncensored model discovery, setup, or prompt
writing. It only reports factual metadata returned by the Civitai API.

## Civitai API

- `CIVITAI_TOKEN` is read from the environment when the endpoint requires auth.
- Responses are cached in `~/.cache/civitfit` for one hour.
- `CIVITFIT_CACHE` overrides the cache directory.
- `CIVITFIT_CACHE_TTL` overrides the TTL in seconds.
- HTTP 429 responses are reported with the server's `Retry-After` value.
- Pagination uses the `nextCursor` value from the response metadata; `--pages`
  controls how many pages `fit` and `search` fetch.

## JSON output

```sh
civitfit --json fit -n 3
civitfit --json info 277058
civitfit --json system
civitfit --json arch
```

The `fit` payload contains `hardware`, `source`, `sort`, `count`, `shown`,
`skipped`, and a `models` array. Each model entry has:

```json
{
  "model": { "id": 277058, "name": "epiCRealism XL", "source": "civitai.com", "url": "..." },
  "version": { "id": 123, "name": "Pure_fix", "base_model": "SDXL 1.0" },
  "arch": "sdxl",
  "tier": 3,
  "comfy": true,
  "bundled": true,
  "file": { "name": "...", "gib": 6.46, "precision": "fp16" },
  "peak_gib": 8.66,
  "mem_ratio": 0.295,
  "status": "comfortable",
  "speed_s": 53.2,
  "steps": 28,
  "notes": []
}
```

`peak_gib`, `mem_ratio` and `speed_s` are estimates. `status` is one of
`comfortable`, `good`, `tight`, `marginal`, `wont_run`, `unsupported`.

## Architecture registry

`data/architectures.json` is embedded into the binary at build time. It contains
22 image-model families with their companion files, native resolution, sampler
steps, and speed constant. Matching is substring-based against Civitai's base
model string, so entries such as `ZImageTurbo`, `Flux.1 D`, `NoobAI` and
`SDXL Lightning` resolve correctly.

To add a family:

1. Add an entry to `data/architectures.json`.
2. Give it a unique `key`, a `tier`, `match` tokens, `params_b`, companions,
   native resolution, steps and `k_sec_per_step`.
3. Run `cargo test` and `civitfit arch`.

## Limitations

- These are estimates, not benchmarks. ComfyUI flags, quantization, offloading
  and the exact text encoder build can move peak memory substantially.
- Civitai's `fp` metadata is reported as-is; it is occasionally inconsistent
  with the actual file size.
- Speed constants are calibrated only for Z-Image so far.
- Licenses are shown from Civitai metadata. Check the model page before
  commercial use; many models are non-commercial or require credit.
- The Civitai API changes and rate-limits. Use `CIVITAI_TOKEN` and the cache;
  do not hammer it.

## License

MIT. See [LICENSE](LICENSE).
