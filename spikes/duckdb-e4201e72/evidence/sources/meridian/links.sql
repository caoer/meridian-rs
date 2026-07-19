-- REAL corpus facts. Wikilink edges; target normalized (basename, no ext,
-- lowercased). `resolved` = the target stem exists as a file.
select src_path, target, resolved from links
