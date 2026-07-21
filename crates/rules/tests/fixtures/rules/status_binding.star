# status_binding — differentiated-value showcase (0003 §3 payload granularity).
#
# Trigger: the "Status" section content changed but the frontmatter `status`
# field did NOT — the two may have drifted.
# Effect: proto.warn (advisory only — the write already happened; never blocks).
#
# This is what a semantic diff buys over "file changed": the rule reasons about
# WHICH section vs WHICH field moved, not merely that the file is dirty.
def on_change(event):
    if "Status" in event.sections_changed and "status" not in event.fields_changed:
        warn(
            message = "The Status section changed but the frontmatter `status` field did not — they may be out of sync.",
            section = "Status",
        )
