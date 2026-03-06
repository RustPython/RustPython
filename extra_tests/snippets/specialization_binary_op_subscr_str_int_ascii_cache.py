def check_ascii_subscr_singleton_after_warmup():
    s = "abc"
    first = None
    for i in range(4000):
        c = s[0]
        if i >= 3500:
            if first is None:
                first = c
            else:
                assert c is first


check_ascii_subscr_singleton_after_warmup()
