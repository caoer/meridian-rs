# LIVE rule — fires whenever the `status` frontmatter field changes (and on a
# new doc whose frontmatter already carries `status`). Over the synthetic
# corpus it fires 3× (doc.md creation, then two status transitions).
def on_change(event):
    if "status" in event.fields_changed:
        notice(message = "status moved on " + event.file)
