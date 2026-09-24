"""Length-bucketing utilities for decision-sidecar batch collation."""

from typing import Any


def bucket_items_by_length(
    items: list[dict[str, Any]],
    max_ratio: float = 1.5,
    max_abs_diff: int = 256,
) -> list[list[dict[str, Any]]]:
    """Group items by sequence length to minimize padding waste during collation.

    Items are sorted by sequence length. A bucket is split when adding an item
    would exceed `max_ratio` (curr_len / min_len) or `max_abs_diff` (curr_len - min_len).
    """
    if not items:
        return []
    if len(items) == 1:
        return [items]

    sorted_items = sorted(items, key=lambda x: len(x["seq"]))
    buckets: list[list[dict[str, Any]]] = []
    curr_bucket = [sorted_items[0]]

    for item in sorted_items[1:]:
        min_len = len(curr_bucket[0]["seq"])
        curr_len = len(item["seq"])
        ratio = curr_len / max(1, min_len)
        abs_diff = curr_len - min_len

        if ratio <= max_ratio and abs_diff <= max_abs_diff:
            curr_bucket.append(item)
        else:
            buckets.append(curr_bucket)
            curr_bucket = [item]

    if curr_bucket:
        buckets.append(curr_bucket)

    return buckets
