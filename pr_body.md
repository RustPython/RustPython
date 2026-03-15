## Description

This PR replaces unsafe unwrap() calls in character conversion functions within crates/sre_engine/src/string.rs with safer alternatives to prevent panics when handling invalid Unicode characters.

## Changes

- lower_unicode(): Use ok().and_then() instead of map().unwrap()
- upper_unicode(): Use ok().and_then() instead of map().unwrap()  
- lower_ascii(): Use ok().map() instead of map().unwrap_or()
- is_word(): Use ok().map() instead of map().unwrap_or()
- is_space(): Use ok().map() instead of map().unwrap_or()
- is_digit(): Use ok().map() instead of map().unwrap_or()
- is_loc_alnum(): Use ok().map() instead of map().unwrap_or()
- upper_locate(): Use ok().map() instead of map().unwrap_or()
- is_uni_digit(): Use ok().map() instead of map().unwrap_or()
- is_uni_alnum(): Use ok().map() instead of map().unwrap_or()

## Why This Matters

These unwrap() calls could cause panics when:
1. char::try_from(ch) fails (invalid Unicode codepoint)
2. to_lowercase().next() or to_uppercase().next() returns None (empty iterator)

By using ok().and_then() and ok().map(), the code now gracefully handles invalid inputs by returning appropriate default values instead of panicking.

## Related Issue

Closes #7434
