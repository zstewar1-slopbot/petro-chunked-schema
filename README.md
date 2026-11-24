# Schema Descriptor Format and Editor for Petroglyph Chuked file formats

This repository provides a `yaml` schema format for describing Petroglyph's chunked file formats, as well as a graphical editor for editing those schemas in conjunction with a view of a real file of some chunked format.

It is built on top of `petro-chunked`, a library for parsing Petroglyph chunk file formats.

It consists of two main crates:

- `petro-chunked-schema` defines the `yaml` schema format for describing chunked file formats. It also provides yaml definitions (and corresponding Rust loaders) for the following known formats:
  
  - [ALO Model files](https://modtools.petrolution.net/file-formats/alo-model-format.html)
  - [ALO Particle files](https://modtools.petrolution.net/file-formats/alo-particle-format.html)
  - [ALA Animation files](https://modtools.petrolution.net/file-formats/ala-animation-format.html)

  The known formats are `include_str`'d into the Rust library when the `known-formats` feature is enabled, and can be retrieved with the `known_formats::alo_model()` and similar functions.

  It also provides a `petro_chunked_schema::dynamic`, a set of types for loading a chunked file into dynamic values, similar to `serde_json::Value`, but for chunked files.

- `petro-chunked-schema-editor` is a [`freya`](https://freyaui.dev/)-based UI designed for editing `petro-chunked-schema` yaml files. It lets you open an existing chunked file and a corresponding schema file (or start a new schema if the schema for the file is not known). It provides tools to go through the chunks and assign them names, apply schema relationships (e.g.chunk M is required, chunk N is optional, chunk X is a map containing chunks Y and Z, chunk W is a sequence of T and U, etc.), and apply typing (chunk X is a struct with fields A, B, C, chunk Y consists of mini-chunks T, U, and V, etc). Features:
  
  - Remembers what files you were working on, returning where you left off on launch.
  - Can be configured with paths to various `GameData` directories, for games and mods.
    - Can load files from Petroglyph MEGA files without needing to extract them.
    - Can run "Validate", which tests the current schema over all files with some file extension in all configured GameData directories, helping find gaps in the schema definition.
  - Emit Rust code for `petro-chunked-derive` that implements the current schema.
  - Hex-editor style view for examining chunk/mini-chunk contents.
    - Allows forcing a detected "mini chunk" block to read as plain data
    - Highlights bytes which are already assigned to structural fields in the schema (color coded), so you can quickly see what's already figured out and what's not.
    - Preview of what selected/subsequent bytes decode to in different formats (floats and ints of different lengths, ascii strings, usual hex-editor things).
   
## Slop Warning

I wanted this tool to help figure out Petroglyph chunked formats which weren't already solved long ago by Mike Lankamp's research, but that's basically just the map file format (which itself is partially known, as it's partially implemented in his [OpenEAW](https://github.com/OpenEAW/OpenEAW) repository). Since that's basically a one off effort, it didn't seem worth it to spend all too much of my human time writing this custom editor, but I have tokens available to burn, so it did seem like it could be worth using them to create this tool.

Therefore, consider yourself forewarned: this app is absolutely vibe-coded. I do some cursory checking on the agent's code for this one, but not much, and certainly not enough to vouch for it beyond that it's not obviously and/or immediately harmful. I would run it on my own PC, but whether that's good enough for you is your call.
