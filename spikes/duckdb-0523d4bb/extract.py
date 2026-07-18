#!/usr/bin/env python3
"""Phase 1 extractor — stdlib only.

Walks the combined corpus (home-wiki + home-wiki-sessions), excluding any
path with a dot-directory component, and emits CSVs:
  files.csv        path, file_rev, size
  nodes.csv        path, span_start, span_end, hpath, node_rev, kind
  links.csv        src_path, target, resolved
  frontmatter.csv  path, key, value
  tags.csv         path, tag

Paths are relative to their corpus root, prefixed with the root's basename:
  home-wiki/...  and  home-wiki-sessions/...

file_rev / node_rev = blake2b(...).hexdigest()[:16]  (blake2b stand-in for blake3).
"""
import csv, hashlib, os, re, sys, time

ROOTS = [
    ("home-wiki", "/Users/Shared/projects/home-wiki"),
    ("home-wiki-sessions", "/Users/Shared/projects/home-wiki-sessions"),
]
OUT = os.path.dirname(os.path.abspath(__file__))

HEADING_RE = re.compile(rb"^(#{1,6}) (.*)$")
WIKILINK_RE = re.compile(r"\[\[([^\]]+)\]\]")
FENCE_RE = re.compile(rb"^\s{0,3}(```|~~~)")
FM_KV_RE = re.compile(r"^([A-Za-z0-9_-]+):\s*(.*)$")

def walk(roots):
    for prefix, root in roots:
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames[:] = sorted(d for d in dirnames if not d.startswith("."))
            for fn in sorted(filenames):
                if fn.endswith(".md"):
                    full = os.path.join(dirpath, fn)
                    rel = prefix + "/" + os.path.relpath(full, root)
                    yield rel, full

def main():
    t0 = time.monotonic()
    files_rows, nodes_rows, links_rows, fm_rows, tags_rows = [], [], [], [], []
    basenames = set()  # lowercase basenames sans .md, for link resolution
    file_list = list(walk(ROOTS))
    total_bytes = 0

    for rel, full in file_list:
        b = os.path.basename(rel)
        if b.lower().endswith(".md"):
            b = b[:-3]
        basenames.add(b.lower())

    for rel, full in file_list:
        try:
            data = open(full, "rb").read()
        except OSError as e:
            print(f"READ-FAIL {rel}: {e}", file=sys.stderr)
            continue
        total_bytes += len(data)
        file_rev = hashlib.blake2b(data).hexdigest()[:16]
        files_rows.append((rel, file_rev, len(data)))

        # -- scan lines with byte offsets; track fences --
        offset = 0
        in_fence = False
        heading_stack = []  # list of (level, title)
        text_lines = []     # (offset, line_bytes, in_fence) for links/frontmatter pass
        for line in data.splitlines(keepends=True):
            stripped = line.rstrip(b"\r\n")
            fence = FENCE_RE.match(stripped)
            if fence:
                in_fence = not in_fence
                text_lines.append((offset, stripped, True))
                offset += len(line)
                continue
            if not in_fence:
                m = HEADING_RE.match(stripped)
                if m:
                    level = len(m.group(1))
                    title = m.group(2).decode("utf-8", "replace").strip()
                    while heading_stack and heading_stack[-1][0] >= level:
                        heading_stack.pop()
                    heading_stack.append((level, title))
                    hpath = " > ".join(t for _, t in heading_stack)
                    node_rev = hashlib.blake2b(stripped).hexdigest()[:16]
                    nodes_rows.append((rel, offset, offset + len(stripped), hpath, node_rev, "heading"))
            text_lines.append((offset, stripped, in_fence))
            offset += len(line)

        # -- links (outside fences) --
        for _, stripped, fenced in text_lines:
            if fenced:
                continue
            try:
                s = stripped.decode("utf-8")
            except UnicodeDecodeError:
                s = stripped.decode("utf-8", "replace")
            for m in WIKILINK_RE.finditer(s):
                target = m.group(1)
                target = target.split("#")[0].split("|")[0].strip()
                if not target:
                    continue
                key = target.rsplit("/", 1)[-1]
                if key.lower().endswith(".md"):
                    key = key[:-3]
                resolved = 1 if key.lower() in basenames else 0
                links_rows.append((rel, target, resolved))

        # -- frontmatter --
        if data.startswith(b"---\n"):
            body = data[4:].decode("utf-8", "replace")
            fm_lines = []
            for ln in body.split("\n"):
                if ln.strip() == "---":
                    break
                fm_lines.append(ln)
            else:
                fm_lines = []  # no closing --- found
            for ln in fm_lines:
                m = FM_KV_RE.match(ln)
                if not m:
                    continue
                key, value = m.group(1), m.group(2).strip()
                fm_rows.append((rel, key, value))
                if key == "tags" and value.startswith("[") and value.endswith("]"):
                    for tag in value[1:-1].split(","):
                        tag = tag.strip().strip("'\"")
                        if tag:
                            tags_rows.append((rel, tag))

    t_extract = time.monotonic() - t0

    t1 = time.monotonic()
    def dump(name, header, rows):
        with open(os.path.join(OUT, name), "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(header)
            w.writerows(rows)
    dump("files.csv", ["path", "file_rev", "size"], files_rows)
    dump("nodes.csv", ["path", "span_start", "span_end", "hpath", "node_rev", "kind"], nodes_rows)
    dump("links.csv", ["src_path", "target", "resolved"], links_rows)
    dump("frontmatter.csv", ["path", "key", "value"], fm_rows)
    dump("tags.csv", ["path", "tag"], tags_rows)
    t_write = time.monotonic() - t1

    print(f"files_ingested={len(files_rows)}")
    print(f"total_bytes={total_bytes}")
    print(f"rows files={len(files_rows)} nodes={len(nodes_rows)} links={len(links_rows)} "
          f"frontmatter={len(fm_rows)} tags={len(tags_rows)}")
    print(f"extract_wall_s={t_extract:.3f}")
    print(f"csv_write_wall_s={t_write:.3f}")
    print(f"total_wall_s={t_extract + t_write:.3f}")

if __name__ == "__main__":
    main()
