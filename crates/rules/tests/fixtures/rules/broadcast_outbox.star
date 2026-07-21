# broadcast_outbox — scenario 1 (the outbox pattern).
#
# Trigger: BROADCAST.md changed.
# Effect: proto.send to the whole fleet, referencing the post-change fingerprint
# so a re-read is anchored to an exact version.
def on_change(event):
    if event.file == "BROADCAST.md":
        send(
            to = ["all"],
            message = "BROADCAST.md changed (" + event.fingerprint_after + ") — re-read it.",
        )
