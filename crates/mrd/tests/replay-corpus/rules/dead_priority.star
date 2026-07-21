# DEAD rule — keys on a `priority` frontmatter field that no document in the
# synthetic corpus ever carries, so it runs on every event yet NEVER fires.
# The replay must report it under "Dead rules (never fired)" — the regression
# gate the CI lane asserts.
def on_change(event):
    if "priority" in event.fields_changed:
        reject(message = "priority is not an allowed field", field = "priority")
