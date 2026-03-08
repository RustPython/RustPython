def check_latin1_subscr_singleton_after_warmup():
    for s in ("abc", "éx"):
        first = None
        for i in range(5000):
            c = s[0]
            if i >= 4500:
                if first is None:
                    first = c
                else:
                    assert c is first


check_latin1_subscr_singleton_after_warmup()
