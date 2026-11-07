class Target:
    value = 0


def mutate_type(iterations, target=Target):
    for value in range(iterations):
        target.value
        target.value = value
    return target.value


# ---

mutate_type(ITERATIONS)
