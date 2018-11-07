x = 0
for i in [1, 2, 3, 4]:
    x += 1

assert x == 4

for i in [1, 2, 3]:
    x = i + 5
else:
    x = 3

assert x == 3
