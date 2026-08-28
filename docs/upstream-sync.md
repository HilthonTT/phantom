# Tracking conduwuit

phantom began as a port of [conduwuit](https://github.com/girlbossceo/conduwuit)
and still tracks it. Substantial portions of the codebase are derived from it,
sometimes verbatim and sometimes adapted. This page is how that relationship is
managed.

## What came from upstream

The parts adapted most directly, and where they now live:

| Upstream area | In phantom |
| :--- | :--- |
| the configuration layer and its example generator | `phantom-core/src/config/`, `phantom-macros` |
| the error type and its construction macros | `phantom-core/src/error/` |
| logging and debug helpers | `phantom-core/src/log/`, `debugger.rs` |
| allocator integration | `phantom-core/src/alloc/` |
| shared utilities | `phantom-core/src/result/`, and the type-named modules beside it |
| the database layer | `phantom-database` |

`NOTICE` is the authoritative list, and is what changes when a new subsystem is
ported.

## Why phantom diverges

Departures are for one of two reasons, and the reason is what decides whether a
divergence is worth keeping.

**Upstream pins an older fork of [ruma](https://github.com/ruma/ruma).**
phantom builds against current crates.io releases, whose APIs have moved since.
Anywhere the port had to be rewritten to compile against a newer ruma — or
newer tracing, or the `rocksdb` crate rather than conduwuit's fork — that is
this reason.

**A subsystem has not been ported yet.** Where upstream's code reaches into
something phantom does not have, the code is trimmed rather than stubbed. A
trimmed call site is honest about what is missing; a stub that silently returns
a default is not.

Two smaller ones recur: `matrix::state_res` is phantom's own implementation
rather than ruma's, which is why the `ruma` dependency deliberately omits the
`state-res` feature; and the module layout was restructured, in particular by
removing the `utils` catch-all — see
[development.md](development.md#conventions).

## The rule for divergences

**Comment at the site.** Where phantom departs from upstream, there is a
comment on the spot saying what changed and why. Not in a changelog, not in a
commit message someone would have to go looking for — next to the code, where
the next person to read it is already standing.

When the divergence is a whole subsystem rather than a call site, add it to
`NOTICE` as well.

## Licence

conduwuit is licensed under the Apache License 2.0, which is why phantom is
too. Attribution, and a summary of what has been changed, are in
[NOTICE](../NOTICE). Keep it accurate: it is the file that makes the
derivation lawful, not a formality.
