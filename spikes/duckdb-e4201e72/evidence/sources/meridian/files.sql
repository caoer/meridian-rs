-- REAL corpus facts. Passthrough of the projected `files` table.
-- Analytical SQL lives in the pages (Evidence style); sources stay thin so the
-- extract dir == the Parquet datalake convergence receipt.
select path, file_rev, bytes from files
