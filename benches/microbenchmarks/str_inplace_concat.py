def concat_in_place():
    value = ""
    for _ in range(ITERATIONS * 20):
        value += "x"
    return value


# ---

concat_in_place()
