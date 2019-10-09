import csv

for row in csv.reader(['one,two,three']):
	[one, two, three] = row
	assert one == 'one'
	assert two == 'two'
	assert three == 'three'

def f():
	iter = ['one,two,three', 'four,five,six']
	reader = csv.reader(iter)

	[one,two,three] = next(reader)
	[four,five,six] = next(reader)

	assert one == 'one'
	assert two == 'two'
	assert three == 'three'
	assert four == 'four'
	assert five == 'five'
	assert six == 'six'

f()
