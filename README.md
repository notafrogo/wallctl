# wallctl

`wallctl` rotates desktop wallpapers from named folders. It supports macOS and Linux Mint’s Cinnamon, MATE, and Xfce desktops, with a small background daemon and a plain INI configuration file.

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
wallctl list
wallctl set 2
wallctl next
wallctl stop
```

The first `wallctl config` creates `~/Library/Application Support/wallctl/config.ini` on macOS and `~/.config/wallctl/config.ini` on Linux, then opens the configuration folder in the system file manager (`open` on macOS, `xdg-open` on Linux). Edit `config.ini` there to change settings or add profiles. Use `wallctl config --path` to print the config file path without opening the folder. On Mint, wallpaper changes use the desktop’s settings service (`gsettings` for Cinnamon and MATE, `xfconf-query` for Xfce), so run `wallctl` from your logged-in desktop session.

```ini
[general]
active_profile = default
restore_on_exit = true

[profile.default]
folder = ~/Pictures/wallpapers
interval = 300
order = alphabetical
mute_video = true
image_scale = fit
```

`image_scale` is profile-specific and accepts `fit`, `fit_vertical`, `fit_horizontal`, or `original` (the default is `fit`).

`order` is either `random` or `alphabetical` (the default); alphabetical order sorts full paths case-insensitively, with a case-sensitive tie-breaker. `interval` is in seconds and can include decimals (for example, `0.1`). Image formats are JPEG, PNG, HEIC, BMP, and WebP. GIFs animate on Linux using the video wallpaper support, which requires `xwinwrap`, `mpv`, `ffmpeg`, and an X11 session; Mint’s Wayland sessions do not support animated GIFs or video wallpapers. On macOS, GIFs are shown as still images and video wallpapers are not supported.

Video changes fade the current video out over 0.45 seconds to a preview frame extracted from the next video, then start the next video. This keeps the desktop background covered during changes, avoids fading again when a video loops, and runs only one video decoder at a time. MPV decoder threads are capped and its file cache is disabled to reduce memory use. Use `wallctl list` to view the active playlist and `wallctl set <index>` to select an entry. Indexes start at 1 and match the order shown by `list`.

## Application API

The daemon exposes a versioned local API over its per-user Unix socket at `~/.config/wallctl/daemon.sock`. The socket accepts one UTF-8 request per connection and returns one JSON response. Socket permissions are restricted to the current user. See [API.md](API.md) for the protocol and client example.

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
