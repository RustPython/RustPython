from random import random, seed
seed(0)

unsorted_list = [random() for _ in range(5 * ITERATIONS)]

# ---

# Setup code only runs once so do not modify in-place
sorted(unsorted_list)
