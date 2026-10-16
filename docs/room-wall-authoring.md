# Author one Room wall operation

The private `orr_remote` example creates an offline proposal for an existing
saved RoomEscapeV1 project. It reads and admits the complete project, validates
one ordinary wall spawn on a private document, and writes a new JSON operations
file. It does not connect to a host, save the scene, or change any project,
package, model-binding, or camera file.

```sh
cargo run -p orr_remote --features room-project --example room_wall -- \
  --project /absolute/path/to/room \
  --guid e_00000100 --name "Interior wall" \
  --center 0.625 0 --half-extents 0.125 0.75 \
  --output /absolute/path/outside-room/wall-ops.json
```

All six options are required exactly once, in any order. There are no positional
arguments, optional switches, abbreviated options, `--flag=value` spellings,
recipe files, or automatic GUID allocation. Unknown, repeated, missing, and
extra arguments fail. Without `room-project`, both default and
`--no-default-features` builds explicitly fail without opening the project or
output. Success exits 0; errors exit 1 with a diagnostic on stderr. Stdout is
empty; the operations are written only to `--output`.

## Values and admission

- `--guid` is an explicit stable GUID accepted by the existing GUID parser:
  `e_` followed by 8 through 32 lowercase hexadecimal digits. It must be absent
  from the admitted saved scene. Existing GUIDs fail even if their data matches
- `--name` is nonempty UTF-8, at most 128 bytes
- `--center` and `--half-extents` take X then Z. Each token is at most 48 bytes
  and uses the existing exact plain-decimal parser: `-?(0|[1-9][0-9]*)(\.[0-9]+)?`
  No exponent, plus sign, whitespace, leading zero, `NaN`, or infinity is accepted
- Decimal conversion uses Q48.16 integer arithmetic, rounding to the nearest
  1/65536 with ties away from zero. The resulting fixed-point value is admitted;
  there is no floating-point conversion or silent clamping. Output uses the
  existing shortest decimal representation that recovers the same raw bits
- Logical options and values require UTF-8. Project and output arguments retain
  native filesystem encoding and are bounded to 4096 encoded bytes

The wall has the existing canonical WALL role (kind 4, ordinal 0), static body
with identity rotation, and standard box collider (wall layer 1, mask 0). Center
Y is fixed at 0.5 and half-height at 1. Canonical components come from the
existing trusted Room setup through public reflection; the authored project is
never replaced with that setup. Projects with zero existing walls are supported.

`PreparedProject::open`, `room_escape::document`, and `EditorDoc::apply` remain
the admission path. Existing Room validation checks the complete numeric
envelope, canonical fields, full player-height coverage, initial player
non-overlap, and the inclusive 64-wall maximum. In the current profile, X/Z
centers are within [-16, 16] and box half-extents within [0.1, 8] after exact
conversion. The helper does not copy these simulation rules into a second
validator. Invalid projects and rejected proposals produce no output file.

## Output path and publication policy

The output is at most 16 KiB and contains exactly one ordinary `spawn` operation
in the existing JSON array format: `op`, explicit `guid`, `name`, and canonical
`components`. There are no additional schema, provenance, or precondition fields.

Choose a new filename outside the admitted project root, in an already-existing
directory. The helper creates no parent directories. POSIX absolute paths and
ordinary relative paths are supported; Windows also supports native
drive-absolute paths. UNC, device, verbatim, and drive-relative paths are not
supported. Every output ancestor must be an existing nonsymlink directory.
Existing destinations of any kind are rejected, including regular files,
hard-link aliases, directories, devices, FIFOs, and dangling symlinks.

Output spellings reject repeated/trailing separators, `.`/`..` components,
control characters, alternate-stream colons, trailing dots/spaces, DOS device
names, and nonnative backslash spellings. This is a deliberately restricted
portable path policy; native non-UTF-8 names otherwise remain supported. A
relative output also requires the working directory's absolute ancestry to
satisfy the policy. For example, use a real directory rather than a symlinked
temporary-directory alias.

After admission, a bounded temporary sibling is created exclusively, written,
and synced. A same-directory hard link publishes the complete file only if the
destination is still absent. Hard-link failures are reported without an
overwrite fallback, and temporary-file cleanup is best effort. A destination
that appears between inspection and publication is never overwritten. The
helper never deletes the requested destination on failure. This policy bounds
ordinary filesystem operations; it does not promise protection from hostile
concurrent directory replacement, mount changes, or crash-durable directory
metadata. Use a directory you control, on a filesystem supporting hard links.

## Adopt and save using the existing verified workflow

Inspect the intended live Room host with `orr status`, `orr scene`, and its
schema, using the existing explicit ERP address and authentication options.
Then use the generated file through the normal proposal/verification route:

```sh
orr apply "Add interior wall" --ops-file /absolute/path/outside-room/wall-ops.json \
  --idle 120 --check 'entities.delta == 1'
```

Choose checks and gameplay inputs appropriate to the intended wall. The example
check only counts entities; it does not prove reachability, intended collision,
key line-of-sight, or visual placement. Review those outcomes before keeping
the edit. Existing undo/redo and proposal history apply normally. Persist an
accepted edit only when intended, using the existing `orr save --write` flow,
then reopen the project through the normal Room consumer.

The output is an offline proposal, not proof that the live document matches the
saved project. There is no project-digest or saved-revision precondition in the
operations file. Live production admission and guarded verify/accept remain
authoritative: a now-duplicate GUID, changed player position causing overlap,
or exhausted wall budget must be rejected by that existing path. Generating a
file neither adopts nor saves it. A new unbound wall uses the existing Room box
presentation; the helper does not add a textured model or an editor toolbar
action.
