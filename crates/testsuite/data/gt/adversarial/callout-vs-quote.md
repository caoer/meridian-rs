# Callout vs blockquote

> [!note]
> A real Obsidian callout, type note.

> [!warning] Titled callout
> Body line of the warning.

> plain blockquote, not a callout

> [!note this is malformed (no closing bracket)
> parser should fall back to blockquote

>[!tip] no space after > (Obsidian still accepts)

> [!faq]- Collapsed (foldable) callout
> Folded body.

> outer quote
> > [!info]
> > nested callout inside a blockquote

Paragraph between quotes.

> [!custom-type] Unknown callout type must still parse as callout syntax
