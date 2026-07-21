# task_conventions — scenario 2 (teach on field change) + view freshness.
#
# Trigger: the frontmatter `status` field changed.
# Effects (in order): proto.notice teaching the valid status values, then
# daemon.refresh_view to mark the derived `tasks` view stale.
#
# Two effects from one rule → exercises per-rule seq (0, 1) and two domains.
def on_change(event):
    if "status" in event.fields_changed:
        notice(
            message = "`status` changed — valid values: todo, in-progress, blocked, review, done.",
        )
        refresh_view(view = "tasks")
