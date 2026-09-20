# ad2edit

An editor for Garry's Mod [AdvDupe2](https://github.com/wiremod/advdupe2) dupes. I made it to build [ACF-3](https://github.com/ACF-Team/ACF-3) tanks and Wiremod contraptions without sitting in the game to do it.

There are two programs. `ad2edit` is the editor, with a 3D view that draws the real models from your game files. `ad2read` does the same jobs from the command line.

## What it does

- Opens and saves AdvDupe2 files, and checks them the way AdvDupe2 does before writing, so it won't hand you a file the game refuses.
- Shows your dupes in a gallery with thumbnails. It keeps the original of anything it saves over.
- Moves, turns and resizes parts. Ammo crates are set by rounds and fuel tanks by litres, using ACF's own maths.
- Adds ACF parts, Wiremod parts and any SProps model.
- Links and wires parts in a node view, and edits Expression 2 chips.
- Tells you what ACF would complain about before you paste.

## What you need

Windows and a Garry's Mod install. The editor reads models from the game and from your workshop addons. None of that content is included here.

## Build it

You need [Rust](https://rustup.rs) 1.95 or newer.

```
cargo build --release --features gui
```

The programs end up in `target/release`. Run the tests with `cargo test --no-default-features`.

## Status

It works on my machine with my dupes and my addon versions. Pasting what it builds hasn't had much testing in game yet, so keep backups.

## Licence

GPL-3.0. This project isn't affiliated with Valve, Facepunch, the ACF Team, Wiremod, or the authors of AdvDupe2, SProps or Primitive.
