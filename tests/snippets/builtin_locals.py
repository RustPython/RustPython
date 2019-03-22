
a = 5
b = 6

loc = locals()

assert loc['a'] == 5
assert loc['b'] == 6

def f():
	c = 4
	a = 7

	loc = locals()

	assert loc['a'] == 4
	assert loc['c'] == 7
	assert not 'b' in loc

