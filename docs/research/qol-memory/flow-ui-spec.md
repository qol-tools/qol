# Launcher flow UI: Enter opens the row's JSON, rendered by its own shape

Continues `phase3-flow-spec.md`. One idea: the launcher gains a generic recursive value renderer,
Enter dives a row into it, and the shape of the data is the shape of the UI. The plugin declares
nothing.

## Today

Every row is `{title, subtitle, copy}`. Enter copies `copy` and closes the window. A memory answer
carries `text`, `layer`, `key`, `cls`, `source_kind`, `source_ts`, `session`, `score`, `margin` and
a `superseded` array; a skill hit carries `id`, `name`, `score`, `section`, `content`, `truncated`,
`status`, `head`, `dirty`. All of it is thrown away at the flatten, and the two look identical in
the list.

The launcher already keeps the whole object: `FlowRow.raw` is the row's `serde_json::Value`.
Nothing new has to cross the wire for this to work.

## The rule

Enter (or `tab`) on a row replaces the list with a **detail view of `row.raw`**, drawn by walking
the value:

- `title`, `subtitle` and `copy` are reserved presentation keys, consumed by the list line and not
  redrawn.
- Everything else is payload and is rendered in the order the plugin emitted it (serde preserves
  declaration order, so the struct's field order is the layout).

Type rules, and there are only these:

| value | drawn as |
|---|---|
| short string (fits one line) | `label   value`, label humanised (`source_kind` becomes `source kind`) |
| long or multi-line string | the label alone, then the text wrapped at the window width, indented |
| number | `label   value`, right aligned, trailing zeros trimmed |
| bool | `label   yes` / `no`, `no` muted |
| null | `label   none`, muted |
| ISO-8601-looking string | rendered as `2026-07-15 19:26:40`, full value on the copy action |
| array of scalars | chips on one wrapped line |
| array of objects | `label  <count>`, then each element indented one level and recursed, foldable |
| object | `label`, then the object indented one level and recursed, foldable |

Depth is expressed by indentation only. Arrays longer than eight elements show the first eight and
`+N more`, which expands with right.

That is the whole vocabulary. It has no memory words in it, so the same view renders a lights
scene, a bluetooth device or a session record with no launcher change.

## What it produces

A memory answer row, from the `Answer` struct's own field order:

```
> where does the clipboard ring live                       answer  .
--------------------------------------------------------------------
  text
    The clipboard ring survives tray restarts because the ring is
    written to the plugin data dir on every append.

  layer         units
  key           7d8917ff17e0d989
  cls           path
  source kind   user
  source ts     2026-07-15 19:26:40
  session       5306df95-d81f-4ec6-9471-0eb1813f8b1b
  score         10.02
  margin        1.9

  superseded  1
    text
      The clipboard ring is rebuilt from scratch on every restart.
    source ts   2026-06-02 11:04:19
--------------------------------------------------------------------
  ctrl-y copy   ctrl-o open   esc back
```

A skill hit row, same renderer, different shape, no extra code:

```
> how does the launcher flow work                           skill  .
--------------------------------------------------------------------
  id            qol-plugin-launcher/qol-plugin-launcher
  name          qol-plugin-launcher
  score         2.5
  section       Source ownership

  content
    | Path | Responsibility |
    |---|---|
    | `src/main.rs`, `src/lib.rs` | Process boundary and public com...
    | `src/app/` | Daemon IPC and retained-process command lifecycle.
    ...

  truncated     no
  hash match    yes
  status        served
  dirty         no
--------------------------------------------------------------------
  ctrl-y copy   ctrl-o open   esc back
```

The verdict row does the same for `gates`, `signals` and `counts`, so "why did it not answer" is one
Enter away instead of invisible: `unit_margin 1.38` sitting next to `UNIT_MARGIN 1.5`.

## Navigation

- `enter` and `tab` dive. `esc` ascends to the list, a second `esc` leaves the flow.
- `up`/`down` scroll the detail; `left`/`right` fold and unfold the object or array containing the
  cursor.
- The prompt bar keeps the query and swaps the `n / m` counter for the row's `kind`, so you always
  know which row you opened.
- Row action keys still fire while dived, against the dived row.

## Plugin side

qol-memory stops flattening. `ask::rows::FlowRow` keeps `title`, `subtitle` and `copy` for the list
line and carries the source object's own fields alongside them: the `Answer` for the answer row, the
`Unit` or `Note` for a recalled row, the `SkillHit` for a skill row, and `verdict`, `confidence`,
`reason`, `gates`, `signals`, `counts` for a leading verdict row. Serialising the existing structs
next to the three presentation keys is the entire change; no new types, no new query.

## Actions while dived

`RowActionSpec` already carries `action`, `input`, `label`, `key` and `when`, and the launcher reads
only `label` on the first entry. Honour the rest so the hint bar above is real:

- `key` binds the action (`enter`, `ctrl-o`, `ctrl-y`); no `key` means Enter.
- `when` gates it on the row: `"<field>"` (present and non-empty), `"<field> == <literal>"`,
  `"<field> != <literal>"`. Anything else fails manifest validation.
- `label` is the hint, shown only for actions whose `when` matches the current row.
- `@copy` and `@detail` are reserved builtins so launcher and plugin actions share one hint bar.
- `then` = `dismiss` (default), `stay`, `refresh`, so an action can loop instead of closing.

With `@detail` bound to Enter by default, a flow that declares nothing still gets the new view.

## Mission check

No host configuration, no new dependency, no first-run cost. Failures stay visible: bad `when`, bad
`key` and unknown action ids fail manifest validation, and a stale skills index is now a visible
`status  stale` field rather than a silent one.

## Sequencing

1. The value renderer plus dive and ascend in the launcher, over `row.raw` as it already arrives.
   Visible immediately with today's rows, which carry `key` and `kind`.
2. qol-memory emits the source objects next to the presentation keys, including a verdict row.
3. Keyed actions, `when`, `then`.

## Non-goals

- Mouse support. The launcher has no mouse handlers anywhere, apps included.
- Markdown rendering. Wrapped monospace; skill bodies are markdown source and read fine as source.
- An expression language in `when`. Three forms are enough; richer logic belongs in the plugin.
- Editing values in the detail view. It is a reader; writes go through declared actions.
