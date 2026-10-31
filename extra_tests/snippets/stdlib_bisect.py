from bisect import bisect_left, bisect_right, insort, insort_left, insort_right

# A key decides where the item goes, but the item is what gets stored.
for insort_fn in (insort, insort_left, insort_right):
    words = ["a", "ccc"]
    insort_fn(words, "bb", key=len)
    assert words == ["a", "bb", "ccc"], (insort_fn.__name__, words)

    numbers = [1, 3]
    insort_fn(numbers, -2, key=abs)
    assert numbers == [1, -2, 3], (insort_fn.__name__, numbers)

    pairs = [(1, "a"), (3, "b")]
    insort_fn(pairs, (2, "x"), key=lambda pair: pair[0])
    assert pairs == [(1, "a"), (2, "x"), (3, "b")], (insort_fn.__name__, pairs)


descending = [3, 1]
insort(descending, 2, key=lambda value: -value)
assert descending == [3, 2, 1], descending


class Tagged:
    def __init__(self, size, tag):
        self.size = size
        self.tag = tag


def sizes_and_tags(items):
    return [(item.size, item.tag) for item in items]


by_size = [Tagged(1, "old"), Tagged(2, "old")]

# On a tie, left goes before the equal element and right goes after it.
left = list(by_size)
insort_left(left, Tagged(2, "new"), key=lambda item: item.size)
assert sizes_and_tags(left) == [(1, "old"), (2, "new"), (2, "old")]

right = list(by_size)
insort_right(right, Tagged(2, "new"), key=lambda item: item.size)
assert sizes_and_tags(right) == [(1, "old"), (2, "old"), (2, "new")]


# The key runs once on the new item, whatever the search does afterwards.
seen = []


def counting_key(value):
    seen.append(value)
    return value


data = [1, 3, 5, 7]
insort(data, 4, key=counting_key)
assert data == [1, 3, 4, 5, 7], data
assert seen.count(4) == 1, seen


# lo and hi bound the search and leave the inserted item alone.
bounded = [3, 1]
insort(bounded, 2, 0, 1, key=lambda value: -value)
assert bounded == [3, 2, 1], bounded


plain = [1, 3]
insort(plain, 2)
assert plain == [1, 2, 3], plain

# The search takes the key value itself, so these two stay where they were.
sizes = ["a", "bb", "ccc"]
assert bisect_left(sizes, 2, key=len) == 1
assert bisect_right(sizes, 2, key=len) == 2
