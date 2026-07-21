# LIVE rule — fires whenever any section's content changes (and on a new doc,
# whose sections all count as new). Over the synthetic corpus it fires 3×
# (doc.md + notes/log.md creation, then the notes/log.md body edit).
def on_change(event):
    if event.sections_changed:
        warn(message = "sections touched in " + event.file)
