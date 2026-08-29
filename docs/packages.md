# Package management

Black Wall Core ships a self-hosted package manager so the OS can be extended
offline or from your own repository.

## Components

- `pkg/anx` — the `anx` CLI: search, install, list, remove, build.
- `pkg/server` — `anx-repo-server`: serves a local package repository.
- `tools/package-builder` — `bw-pkg`: builds `.anxpkg` archives from recipes.
- `tools/updater` — `anxd`: automatic update daemon.

## Repository configuration

Configuration lives at `/etc/anx/anx.toml`. The default repository URL is a
neutral local default:

```toml
default_channel = "stable"

[[channels]]
name     = "stable"
url      = "http://localhost:8484"
priority = 10
enabled  = true
```

Point `url` at your own repository server (e.g. an `anx-repo-server`
instance). No hosted/private repository is hard-coded.

## Building a package

```sh
bw-pkg build path/to/recipe.toml
```

The package-builder supports many archive formats (including `.zip`) and
produces a `.anxpkg` artifact that `anx install` can install. See
`tools/package-builder` and its example recipes for details.

## Publishing locally

1. `anx-repo-server` serves a directory of `.anxpkg` files over HTTP.
2. Configure a channel pointing at it in `/etc/anx/anx.toml`.
3. `anx update` and `anx install <pkg>` from the configured channel.
