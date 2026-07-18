# Anchor edge cases

Normal paragraph with a line-tail anchor ^plain

^lonely-anchor-on-own-line

Two anchors same line: first ^first second ^second

- list item with anchor ^in-list
- [ ] task with anchor ^in-task

> quote line with anchor ^in-quote

| a | b |
|---|---|
| 1 | 2 |

^after-table

```
code fence containing ^not-an-anchor (must NOT extract)
```

Inline code with `^not-an-anchor-either` in backticks.

Unicode anchor ^café-锚

Not anchors: word^caret-glued-to-word, lone caret ^ (nothing after),
and mid-sentence ^mid then more text (Obsidian: block ref must be line-tail).

Heading ref target below:

## Target heading ^heading-anchor
