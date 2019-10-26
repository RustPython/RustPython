import random 

random.seed(1234)

# random.randint
assert random.randint(1, 11) == 8

# random.shuffle
left = list(range(10))
right = [2, 7, 3, 5, 8, 4, 6, 9, 0, 1]
random.shuffle(left)

assert all([l == r for l, r in zip(left, right) ])

# random.choice
assert random.choice(left) == 5

# random.choices 
expected = ['red', 'red', 'black', 'red', 'black', 'black']
result = random.choices(['red', 'black', 'green'], [18, 18, 2], k=6)
assert all([l == r for l, r in zip(expected, result) ])

# random.sample
sampled = [2, 1, 0]
assert all([l == r for l, r in zip(random.sample(l, 3), sampled) ])

# TODO : random.random(), random.uniform(), random.triangular(),
#        random.betavariate, random.expovariate, random.gammavariate,
#        random.gauss, random.lognormvariate, random.normalvariate,
#        random.vonmisesvariate, random.paretovariate, random.weibullvariate
