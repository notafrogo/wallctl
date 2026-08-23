# wallctl

`wallctl` rotates desktop wallpapers from named folders. It is designed for macOS and Cinnamon desktops, with a small background daemon and a plain INI configuration file.

## Install

Install via brew: (not done yet)

```sh
brew install notafrogo/tap/wallctl
```

## Quick start

```sh
wallctl config
wallctl start
wallctl status
wallctl next
wallctl stop
```

The first `wallctl config` creates `~/Library/Application Support/wallctl/config.ini` on macOS and `~/.config/wallctl/config.ini` on Linux. Put images in the default folder or change the profile’s `folder` value, then run `wallctl reload`.

```ini
[general]
active_profile = default
restore_on_exit = true

[profile.default]
folder = ~/Pictures/wallpapers
interval = 300
order = random
mute_video = true
```

`order` is either `random` or `alphabetical`; `interval` is in seconds. Static image formats are JPEG, PNG, HEIC, BMP, WebP, and GIF. Linux video wallpapers require `xwinwrap` and `mpv`; video wallpapers are intentionally rejected on macOS instead of launching an unattached player window.

The daemon remembers the pre-`wallctl` wallpaper and restores it on `stop` when `restore_on_exit` is true. It does not edit macOS’s private wallpaper database, so it will not delete or corrupt entries managed by the System Settings wallpaper picker.

## Development

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets -- -D warnings
```

## Releases and Homebrew

Push a tag matching the Cargo version, such as `v0.2.0`. The release workflow checks the tag/version match, runs quality gates, builds separate Intel and Apple Silicon archives, publishes checksums, and creates the GitHub release.

To have the workflow open a PR in your Homebrew tap, configure these in the source repository:

- Repository variable `HOMEBREW_TAP_REPOSITORY`, for example `notafrogo/homebrew-tap`.
- Repository secret `HOMEBREW_TAP_TOKEN`, a fine-grained token with **Contents: Read and write** and **Pull requests: Read and write** on that tap repository.

The workflow updates `Formula/wallctl.rb` and opens a reviewable pull request; it never pushes directly to the tap’s default branch. Publishing to `homebrew/core` remains a separate, maintainer-reviewed contribution.
