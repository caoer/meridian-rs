---
title: Backlinks explorer
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=info>

**as_of_root** <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/> · **data: REAL corpus facts**

</Alert>

Find-references for any page. Pick a link **target** (a page's basename,
lowercased, no `.md` — the normalized form wikilinks resolve to). Defaults to
`llm_wiki`. This is the engine's `backlinks(target)` fact op rendered for humans —
the *same* projection the wire face serves to an agent doing a rename edit-set.

```sql top_targets
select target, count(*) as inbound
from meridian.links
where resolved
group by target
order by inbound desc, target
limit 40
```

<Dropdown data={top_targets} name=target value=target defaultValue="llm_wiki">
  <DropdownOption value="llm_wiki" valueLabel="llm_wiki (default)"/>
</Dropdown>

```sql summary
select
  '${inputs.target.value}' as target,
  count(*) filter (where resolved) as resolved_backlinks,
  count(*) filter (where not resolved) as dangling_refs,
  count(*) as total_refs
from meridian.links
where target = '${inputs.target.value}'
```

<BigValue data={summary} value=resolved_backlinks title="Resolved backlinks"/>
<BigValue data={summary} value=dangling_refs title="Dangling refs to this name"/>
<BigValue data={summary} value=total_refs title="Total references"/>

## Pages linking to <Value data={summary} column=target/>

```sql backlinks
select
  src_path as source_page,
  resolved
from meridian.links
where target = '${inputs.target.value}'
order by resolved desc, src_path
```

<DataTable data={backlinks} rows=20 search=true>
  <Column id=source_page wrap=true/>
  <Column id=resolved/>
</DataTable>

<Details title="SQL — the backlinks fact op">

The core query is a single indexed lookup on `links.target`:

```sql
select src_path from links where target = ? and resolved;
```

At rung 5 this is an **engine fact op** (RAM, wire) — an agent renaming a page
needs the exact edit-set of inbound references, not a dashboard. The parameterized
page above is the *human* face of the identical projection.

</Details>
