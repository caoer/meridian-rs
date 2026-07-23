# join_arming — scenario 3 (arm a newly-created agent file).
#
# Trigger: a new file under agents/ (no prior fingerprint = a creation).
# Effects (in order): proto.send the conventions digest to the new agent, then
# proto.remind it to set its status one-liners.
def on_change(event):
    if event.file.startswith("agents/") and event.fingerprint_before == "":
        send(
            to = [event.file],
            message = "Welcome. Conventions: claim tasks atomically; move your card as you go; keep your agent status current.",
        )
        remind(
            message = "Set your manifest + status one-liners before your first report.",
        )
