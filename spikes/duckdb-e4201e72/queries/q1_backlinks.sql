-- Q1 backlinks of a hot page (LLM_WIKI.md): pages that link TO it.
-- Wikilinks are stored normalized (basename, no ext, lowercased) in links.target.
SELECT src_path
FROM links
WHERE target = 'llm_wiki'
ORDER BY src_path;
