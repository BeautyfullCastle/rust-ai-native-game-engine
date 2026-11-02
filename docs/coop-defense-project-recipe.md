# Portable CoopDefense deployment recipe

This optional web-authoring CLI remembers the inputs for one offline HTML
release. It does not execute a build or install a package, change the simulation,
or replace the packer's validation. First produce a trusted local web build with
`tools/build_coop_defense_web.sh` (the complete module-copy fix is PR350).

Save `project.json` next to the build and authored inputs:

```json
{
  "format": "orr_coop_project",
  "version": 1,
  "build": "dist",
  "scene": "scenes/mission.yaml",
  "animations": "art/animations.json",
  "audio": "sound/bank.json"
}
```

Only `format`, `version`, and `build` are required. Omit unused optional fields;
`null` is not an omission. Then run from any working directory:

```sh
python3 tools/build_coop_defense_recipe.py project.json --output releases/game.html
```

The output parent must already exist. Output is an explicit command-line path
(relative to the working directory), never a recipe field, and must not exist.
Existing destinations, including dangling symlinks, are never overwritten.
The existing packer publishes via an exclusive hard link after admission and
cleans its temporary sibling on failure.

## Relocation and boundaries

All recipe input paths are relative to the recipe's parent, not the shell's
working directory. Move that whole directory with its build and authored inputs
to relocate a project. Absolute paths, backslashes, colons, empty components,
`.` and `..`, and symlink components are rejected. Inputs must be regular files;
the build must be a real directory. Referenced animation/audio assets remain
subject to their existing strict packer admission and size limits.

Recipes are bounded to 8 KiB UTF-8 JSON, each path to 1024 characters. Unknown or
duplicate keys, nonfinite JSON, and unsupported format/version fail. No shell,
executable, remote download, or environment-expansion fields are supported.
The trusted locally built WASM still runs in the existing Node scene validator
when a scene is supplied; this is not a WASM sandbox. Use trusted, quiescent
project directories, not concurrently adversarially modified trees.

The CLI is a composition convenience, not a new asset format. Scene checksum,
animation image/frame admission, audio PCM/digest admission, module graph checks,
and the output publisher are unchanged. Successful packaging proves neither
browser gameplay nor physical audio/GPU behavior. Core engine and package
manager are unchanged.
