# status_log — the acknowledged-limitation escape (0003 §4).
#
# Cursor replay collapses intermediate transitions: todo→review→done re-emits as
# todo→done, so "entered review" never fires over the wire. A rule that must
# record EVERY transition writes it INTO the tree at write time — disk carries the
# history the wire never does.
#
# Trigger: the `status` field changed.
# Effect: md.append_section — a durable log line (a cascading, depth-capped effect).
def on_change(event):
    if "status" in event.fields_changed:
        append_section(
            section = "Log",
            content = "status changed at " + event.fingerprint_after,
        )
