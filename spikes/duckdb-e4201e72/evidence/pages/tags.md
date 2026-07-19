---
title: Tag aggregation
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=info>

**as_of_root** <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/> · **data: REAL corpus facts**

</Alert>

Corpus-wide frontmatter `tag` frequency. One row per (page, tag); a page with
three tags contributes three rows.

```sql tag_totals
select
  count(*) as tag_assignments,
  count(distinct value) as distinct_tags,
  count(distinct path) as tagged_pages
from meridian.frontmatter
where key='tag'
```

<BigValue data={tag_totals} value=distinct_tags title="Distinct tags" fmt=num0/>
<BigValue data={tag_totals} value=tag_assignments title="Tag assignments" fmt=num0/>
<BigValue data={tag_totals} value=tagged_pages title="Tagged pages" fmt=num0/>

```sql tags
select value as tag, count(*) as n, count(distinct path) as pages
from meridian.frontmatter
where key='tag'
group by value
order by n desc, tag
limit 50
```

<BarChart data={tags.slice(0,25)} x=tag y=n title="Top 25 tags by frequency" swapXY=true/>

## Top-level tag prefixes (taxonomy shape)

```sql prefixes
select
  split_part(value, '/', 1) as prefix,
  count(*) as n,
  count(distinct value) as distinct_tags
from meridian.frontmatter
where key='tag'
group by 1
order by n desc
limit 20
```

<DataTable data={prefixes} rows=12/>

## Full ranking (top 50)

<DataTable data={tags} rows=15 search=true/>

<Details title="SQL">

```sql
select value as tag, count(*) as n
from frontmatter where key='tag'
group by value order by n desc limit 50;
```

</Details>
