---
title: Hub / coverage — inbound-link distribution
---

```sql asof
select * from wiki.asof
```

<Alert status=info>

**as_of_root** **{asof[0].root_hex}** — snapshot built {asof[0].root_built_at}, source `wiki-snap.duckdb` (CoW clone, read-only). **Provenance: REAL corpus facts.**

</Alert>

Inbound-link (in-degree) distribution over the resolved link graph — the
coverage half of r3's `QC` degree-centrality feed. Hubs are the pages the corpus
leans on; the long tail is everything linked once or twice.

## Top hubs by inbound links

```sql hubs
select target as page_key, count(*) as in_deg
from wiki.links
where resolved
group by target
order by in_deg desc
limit 25
```

<BarChart data={hubs} x=page_key y=in_deg swapXY=true title="Top 25 hubs (inbound resolved links)"/>

## In-degree distribution (how many pages have N inbound links)

```sql dist
with indeg as (
  select target, count(*) as in_deg
  from wiki.links where resolved
  group by target
)
select in_deg, count(*) as pages
from indeg
group by in_deg
order by in_deg
limit 60
```

<BarChart data={dist} x=in_deg y=pages title="Inbound-link count → number of pages (power-law tail)"/>

<DataTable data={hubs} rows=10 search=true>
  <Column id=page_key title="Hub (link key)"/>
  <Column id=in_deg title="inbound links" contentType=colorscale/>
</DataTable>
