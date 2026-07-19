-- Write-batch root transitions (root_before -> root_after per batch). Documented
-- extension over the protocol minimum: expresses the root TRANSITION the
-- rootstamp demo needs. Values here are from the synthetic rootstamp step.
select seq, root_before, root_after, ts from batches
