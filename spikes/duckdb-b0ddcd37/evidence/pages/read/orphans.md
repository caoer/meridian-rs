---
title: Orphans & dangling links
---

```sql asof
select * from wiki.asof
```

<Alert status=info>

**as_of_root** **{asof[0].root_hex}** — snapshot built {asof[0].root_built_at}, source `wiki-snap.duckdb` (CoW clone, read-only). **Provenance: REAL corpus facts.**

</Alert>

Two anti-joins over the link graph. **Orphans** = pages with no inbound resolved
link (r3 `Q5`). **Dangling** = link targets that resolve to no file.

```sql counts
select
  (select count(*) from wiki.files f
     where not exists (select 1 from wiki.links l where l.target = f.norm_key and l.resolved)) as orphans,
  (select count(*) from wiki.files) as total_files,
  (select count(*) from wiki.links where not resolved) as dangling_links,
  (select count(distinct target) from wiki.links where not resolved) as dangling_targets
```

<BigValue data={counts} value=orphans title="orphan pages (no inbound link)" fmt='#,##0'/>
<BigValue data={counts} value=total_files title="total files" fmt='#,##0'/>
<BigValue data={counts} value=dangling_links title="dangling link rows" fmt='#,##0'/>
<BigValue data={counts} value=dangling_targets title="distinct dangling targets" fmt='#,##0'/>

## Dangling targets (most-referenced broken links)

```sql dangling
select target as broken_target, count(*) as refs
from wiki.links
where not resolved
group by target
order by refs desc
limit 200
```

<DataTable data={dangling} rows=12 search=true>
  <Column id=broken_target title="Dangling target (no file resolves)"/>
  <Column id=refs title="Referencing links" contentType=colorscale/>
</DataTable>

## Orphan pages (sample of 200)

```sql orphans
select f.path
from wiki.files f
where not exists (select 1 from wiki.links l where l.target = f.norm_key and l.resolved)
order by f.path
limit 200
```

<DataTable data={orphans} rows=12 search=true/>
