# Irminsul auto-artifactarium fork

This directory contains the source fork used by Irminsul, based on
[`konkers/auto-artifactarium` at `abbbf7a3f58846512250a789a76be2d3cdb115c1`](https://github.com/konkers/auto-artifactarium/tree/abbbf7a3f58846512250a789a76be2d3cdb115c1).
The upstream MIT license and history are retained in
[`Seeker14491/auto-artifactarium`](https://github.com/Seeker14491/auto-artifactarium).
Irminsul pins a commit of this repository as a Git submodule at
`vendor/auto-artifactarium`, keeping it as a Cargo workspace member and path
dependency. Clone Irminsul recursively or run
`git submodule update --init --recursive` before building.

## Added protocol API

`auto_artifactarium::position` provides:

- The captured 7.0 profile, custom JSON profiles, and profile validation.
- Bounded protobuf parsing with exact float32 coordinate preservation.
- Build negotiation and ordered decoding of nested batched commands.
- Typed movement, team, identity, entity replacement, and invalidation events.

The built-in profile was validated on
`OSRELWin7.0.0_R47805902_S47829085_D48139903` and
`OSRELWin7.0.0_R47805902_S47829085_D48215702`. The September 15 update was validated
against 2,148 fresh packet bodies: 165 accepted positions and zero errors with the
same mappings. Its `matching: "data_revision"` policy accepts other numeric `D`
revisions when the entire release/platform, `R`, and `S` match. This is a bounded
compatibility assumption; future layouts are not guaranteed. Unknown coordinate
fields fail validation instead of becoming zero components. Omitted zero axes
and an empty zero vector retain protobuf semantics.

Custom build rules default to `matching: "exact"` when omitted. Multiple guards
for one command still accept any matching rule. `Decoder::observed_build()` and
`build_match()` distinguish the actual build from the profile reference, including
rejected builds; reset clears them and failed nested batches roll back changes.
Rejection diagnostics distinguish received, missing, and malformed values.

`profiles/position-7.0.json` is the embedded default and a reusable mapping template.
Command IDs, paths, filters, and axis tags are data rather than compiled protocol
constants. Irminsul can hot-load validated profiles or select one at startup with
`--position-profile`. Synthetic shuffled-layout tests cover remapping inner and
outer fields without code changes. Major versions with new event semantics or
encodings still need decoder changes; automatic mapping discovery is not provided.

The upstream 7.1 update changes inventory/avatar command IDs and their outer
list fields. It does not supply 7.1 position mappings. The built-in position
profile remains guarded to the validated 7.0 build family until a new profile
has been validated against 7.1 captures.

Reviewed [upstream PR #1](https://github.com/konkers/auto-artifactarium/pull/1): its
item/avatar heuristics find outer repeated fields but retain fixed inner schemas.
They do not disambiguate movement vectors or identify the local avatar. The
position decoder therefore keeps explicit validated mappings.

`position::Decoder::receive(command_id, body)` accepts a decrypted body without
`PacketHead`. `None` means an unknown command or traffic blocked by the build
guard. `Some(packet)` retains event order and identity-list boundaries, and counts
successfully decoded leaf commands separately from unknown commands. A
movement with `motion: Ok(None)` has no transmitted position.

Every movement rule requires `tick_rate_hz` (integer 1–1,000,000,000). The decoder
propagates it with `Motion::tick_rate_hz` so consumers can convert tick differences
to seconds without using packet delivery timing. The captured 7.0 profile uses
1,000 Hz, validated against two recordings spanning approximately 34 and 65 seconds.
Custom mappings must verify their clock rate; profiles without one are rejected.

The library does not select a local player. It returns all matching candidates.
Candidate fields carry `Result` values so a consumer can ignore malformed or
incomplete foreign-player fields before requiring the selected player's data.
Check owner/entity selectors before inspecting candidate payloads. A consumer
must handle errors in selected payloads and apply each outer packet atomically.

Irminsul clones its protocol and tracker state before decoding/applying a packet,
then commits them together. Tracking, receipt timestamps, point saving, reference
heights, CSV, and UI remain in Irminsul. Existing inventory/avatar/achievement
decoding is preserved. Inherited source received formatting and mechanical Clippy
cleanup so the local workspace passes Irminsul's strict lint check.

## Maintenance

Run `cargo test --release --locked --workspace` from the Irminsul repository root.
In a standalone clone of this fork, run `cargo test --release`,
`cargo fmt --all --check`, and `cargo clippy --release --all-targets -- -D warnings`.
The library's protocol tests run without any Irminsul tracker or UI. Irminsul's
root Cargo.lock pins its build dependencies; this library does not commit a lockfile.

For upstream updates, fetch `https://github.com/konkers/auto-artifactarium.git`
into an `upstream` remote and review/merge its changes on a working branch,
retaining `src/position` and its module export. Update this provenance record
when adopting a newer upstream base. Run workspace tests and compare the private
capture replay before accepting the update.

Commit and push decoder changes here before updating Irminsul's submodule pointer.
The parent repository records the exact commit; checking out Irminsul must not
automatically advance the decoder to a branch tip. Make changes on a branch in
the submodule, since normal submodule checkouts have a detached HEAD. See
Irminsul's `docs/decoder-submodule.md` for the coordinated workflow.
