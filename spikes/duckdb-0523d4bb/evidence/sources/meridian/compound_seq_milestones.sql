-- seq milestones as stand-ins for root-A / root-B (real roots table has no seq mapping in the spike)
SELECT 'p00 (root-A stand-in)' AS label, min(seq) AS seq FROM changes
UNION ALL SELECT 'p50 checkpoint', quantile_disc(seq, 0.50) FROM changes
UNION ALL SELECT 'p90 checkpoint', quantile_disc(seq, 0.90) FROM changes
UNION ALL SELECT 'p100 (root-B stand-in)', max(seq) FROM changes
ORDER BY seq
