//! `orr help` and `orr <command> --help`.

const OVERVIEW: &str = "orr: command line for a running Orrery host (editor with --erp, or orr_remote_host)

usage: orr [--erp URL] [--token T] [--json] <command> [args]

Look
  status                       host, game, mode, tick, checksum, unsaved changes, proposals, clients
  scene                        entities (GUID, name, components) and singletons
  get <entity> [Comp[.path]]   component values, exact decimals
  schema [type] | --types      the value schema of the game's types

Change
  set <entity> Comp.path=value...   edit fields (one undo entry)
  spawn | despawn | rename | add | remove
  apply <label> <ops...> --check <rule>   propose, verify, accept if it passes (recommended)
  propose | verify | accept | reject | proposals | diff      the same, step by step
  history | undo | redo

Other
  sim start|stop|play|pause|step|seek|speed|state
  activity [-f]                what agents did on the host (a live feed with -f)
  save [--write]               scene YAML, or write the host's scene file
  agents-md                    the guide for AI agents, generated from the host
  help [command]

Connection: --erp URL or $ORR_ERP (default ws://127.0.0.1:7777); --token T or $ORR_ERP_TOKEN
(no token = a host started with --dev-no-auth). --json prints the raw structured result.
No host yet:  orr_editor --erp 127.0.0.1:7777 --erp-dev      or headless:  orr_remote_host --dev-no-auth
Exit codes: 0 ok, 1 command failed, 2 usage error, 3 cannot connect or authenticate, 4 checks failed.
Entities are GUIDs (e_0000000e) or display names; component names may be short (Body.pos) when unambiguous.
Values are JSON when they parse (pos=[6,18], angle=0.5), else bare strings (kind=dynamic).";

/// (name, usage line, help text)
const COMMANDS: &[(&str, &str, &str)] = &[
    (
        "status",
        "orr status",
        "orr status

Host, game, build id, mode (edit or play), tick, checksum, unsaved changes, open proposals, history and connected clients.
Start here to see that the host is there and what state it is in.

Example:
  orr status
  orr status --json",
    ),
    (
        "scene",
        "orr scene [--components] [--filter <text>] [--has <Type>] [--limit N] [--offset N]",
        "orr scene [--components] [--filter <text>] [--has <Type>] [--limit N] [--offset N]

Lists the entities (GUID, display name, component types) and the singletons.
  --components    also print every component value
  --filter TEXT   only entities whose name contains TEXT
  --has TYPE      only entities that have this component (short names work)
  --limit/--offset  page through a big scene (default limit 200)

Examples:
  orr scene --filter body_0
  orr scene --has Body --components --limit 5",
    ),
    (
        "get",
        "orr get <entity|name|@Singleton> [Component[.path]] [--proposal pN]",
        "orr get <entity|name|@Singleton> [Component[.path]] [--proposal pN]

Reads component values in the scene format (fixed-point numbers as exact decimals, vectors as [x, y]).
The entity is a GUID or an exact display name (ambiguous names list the matches). `@Scene` reads a singleton.
--proposal pN reads the entity as it would be after that proposal is accepted.

Examples:
  orr get body_05
  orr get body_05 Body.pos
  orr get e_0000000e orr_physics::Body
  orr get @Scene",
    ),
    (
        "schema",
        "orr schema [type] | --types",
        "orr schema [type] | --types

The JSON Schema of the game's component and singleton types: fields, ranges, docs.
--types lists just the type names and one-line docs. With a type name (short names work) it prints that type only.

Examples:
  orr schema --types
  orr schema Body",
    ),
    (
        "set",
        "orr set <entity> <Component.path>=<value> [...more]",
        "orr set <entity> <Component.path>=<value> [<Component.path>=<value>...]

Direct edit of one or more fields; all assignments together are ONE undo entry (agent origin).
Values are JSON when they parse, else bare strings or decimals. Short component names work when unambiguous.
For anything bigger than a small edit use `orr apply`, which verifies before it changes the scene.

Examples:
  orr set body_05 Body.pos=[6,18]
  orr set hero orr_physics::Body.kind=dynamic Body.angle=0.5
  orr set e_0000000e Body.vel=[0,0]",
    ),
    (
        "spawn",
        "orr spawn [--name <name>] [Component=<json>...]",
        "orr spawn [--name <name>] [Component=<json>...]

Creates an entity with the given whole components; prints its new GUID.

Example:
  orr spawn --name crate 'Body={\"pos\":[0,20],\"kind\":\"dynamic\"}'",
    ),
    ("despawn", "orr despawn <entity>", "orr despawn <entity>\n\nDeletes an entity (refused while another entity points at it).\n\nExample:\n  orr despawn crate"),
    ("rename", "orr rename <entity> <name>", "orr rename <entity> <name>\n\nSets the display name.\n\nExample:\n  orr rename body_05 hero"),
    (
        "add",
        "orr add <entity> <Component> [json]",
        "orr add <entity> <Component> [json]\n\nAdds a component (the type default when no value is given).\n\nExample:\n  orr add hero PaddleTag",
    ),
    ("remove", "orr remove <entity> <Component>", "orr remove <entity> <Component>\n\nRemoves a component.\n\nExample:\n  orr remove hero PaddleTag"),
    (
        "propose",
        "orr propose <label> <ops...> | --ops-file ops.json | -",
        "orr propose <label> <ops...> | --ops-file ops.json | -

Stages edits as a PROPOSAL on a private copy: the scene is not changed. Prints the id, a summary and the diff.
Then `orr verify <id> --check ...` and `orr accept <id>` (or `orr reject <id>`); `orr apply` does all of it in one step.

Op syntax:
  set <entity> <Comp.path>=<value>...   sset <Singleton.path>=<value>...
  rename <entity> <name>   despawn <entity>   add <entity> <Comp> [json]   remove <entity> <Comp>
  spawn [--name n] [Comp=<json>...]
  or a JSON op list from --ops-file or stdin (`-`).

Examples:
  orr propose \"lift hero\" set hero Body.pos=[6,18] rename body_05 hero
  echo '[{\"op\":\"despawn\",\"entity\":\"e_0000000e\"}]' | orr propose \"drop it\" -",
    ),
    (
        "verify",
        "orr verify [<proposal>] [--bot N | --idle N | --last-play | --replay f.orrp] [--seed S] [--check <rule>]...",
        "orr verify [<proposal>] [--bot N | --idle N | --last-play | --replay f.orrp] [--seed S] [--check <rule>]...

Replays the scene without and with the proposal, deterministically, on the same inputs, and compares checksums and metrics.
Without a proposal it runs the scene alone (baseline metrics: use them to choose thresholds).
Prints pass or fail per check with the reason, and the metrics that changed. Exit 4 if a check fails.

Checks: <metric>[.start|final|min|max|delta] <|<=|==|!=|>=|> <number>, `base:` prefix for the run without the change,
no_divergence, no_divergence_before <tick>, recording_matches.
Inputs: --bot N ticks of scripted players (default 300; --seed S, --players P), --idle N, --last-play, --replay file.

Examples:
  orr verify
  orr verify p1 --check \"lost_bodies.max == 0\" --check \"mean_height >= 2.5\"
  orr verify p1 --idle 600 --check no_divergence_before 1",
    ),
    (
        "accept",
        "orr accept <proposal>",
        "orr accept <proposal>\n\nApplies a proposal as ONE undoable history entry. Needs the `approve` capability. Conflict: nothing changes.\n\nExample:\n  orr accept p1",
    ),
    ("reject", "orr reject <proposal>", "orr reject <proposal>\n\nDiscards a proposal; the scene is untouched.\n\nExample:\n  orr reject p1"),
    ("proposals", "orr proposals [<proposal>]", "orr proposals [<proposal>]\n\nLists open proposals (people at the editor make them too), or shows one in full.\n\nExample:\n  orr proposals"),
    ("diff", "orr diff <proposal>", "orr diff <proposal>\n\nThe changes a proposal would make, and the diff of the scene text.\n\nExample:\n  orr diff p1"),
    (
        "apply",
        "orr apply <label> <ops...> [--check <rule>]... [--bot N | --idle N | --last-play | --replay f] [--keep]",
        "orr apply <label> <ops...> [--check <rule>]... [--bot N | --idle N | --last-play | --replay f] [--seed S] [--keep]

The one-shot workflow: propose, verify with the given checks, accept if all pass; otherwise reject and exit 4 with the report.
Default check when none is given: `lost_bodies.max == 0`, if the game reports that metric (otherwise no check).
--keep leaves a failed proposal open instead of rejecting it. Ops: see `orr propose`; --ops-file f.json or `-` for a JSON list.
The accepted change is one history entry; `orr undo` takes it back.

Examples:
  orr apply \"lift hero\" set hero Body.pos=[6,18] --check \"lost_bodies.max == 0\"
  orr apply \"drop crate\" despawn crate --check \"entities.final == 48\" --bot 600
  orr apply \"heavier\" set body_05 Body.mass=4 --keep",
    ),
    ("history", "orr history [-n N]", "orr history [-n N]\n\nThe undo history, oldest first: id, label, who made it (user or agent:<name>), ops, undone. -n N: only the last N.\n\nExample:\n  orr history -n 5"),
    ("undo", "orr undo", "orr undo\n\nTakes back the last history entry (a whole proposal or edit at once), whoever made it. Not while a play session runs."),
    ("redo", "orr redo", "orr redo\n\nRepeats the last undone entry."),
    (
        "sim",
        "orr sim start [--players N] | stop | play | pause | step [N] | seek <tick> | speed <x> | state",
        "orr sim start [--players N] | stop | play | pause | step [N] | seek <tick> | speed <x> | state

Drives a play session (a copy of the scene; the scene document is untouched).
  start   begin paused     step N   run N ticks now (default 1)   seek T   go to a recorded tick
  play / pause   run by the wall clock or not      speed X   wall-clock speed, e.g. 0.5 or 2
  stop    end it; the recording stays for `orr verify --last-play`     state   mode, tick, checksum

Examples:
  orr sim start
  orr sim step 60
  orr sim seek 30
  orr sim state
  orr sim stop",
    ),
    (
        "activity",
        "orr activity [--since <seq>] [--reads] [-n N] [-f]",
        "orr activity [--since <seq>] [--reads] [-n N] [-f|--follow]

The host's activity feed: what every client's requests did, one line each, with old and new values of field writes.
  --since SEQ   only entries after this sequence number   --reads   also read-only requests   -n N   the newest N (default 50)
  -f / --follow   stream new entries until Ctrl-C (watch what an agent does, like the editor's Activity tab)

Examples:
  orr activity
  orr activity -f --reads",
    ),
    (
        "save",
        "orr save [--write]",
        "orr save [--write]\n\nPrints the scene as YAML. With --write the host writes its scene file and marks the document saved (needs scene_edit).\n\nExamples:\n  orr save > backup.scene.yaml\n  orr save --write",
    ),
    ("agents-md", "orr agents-md", "orr agents-md\n\nPrints the guide for AI agents, generated from this host (engine version, build id, types, metrics, check grammar).\n\nExample:\n  orr agents-md > AGENTS.md"),
];

/// The overview printed by `orr help`.
pub fn overview() -> &'static str {
    OVERVIEW
}

/// The help of one command, with the shared op and check notes where they apply.
pub fn command(name: &str) -> Option<&'static str> {
    COMMANDS.iter().find(|(n, _, _)| *n == name).map(|(_, _, t)| *t)
}

/// The one-line usage of a command.
pub fn usage_line(name: &str) -> Option<&'static str> {
    COMMANDS.iter().find(|(n, _, _)| *n == name).map(|(_, u, _)| *u)
}
