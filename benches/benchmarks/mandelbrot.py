#!/usr/bin/env python3
# coding: utf-8

w = 50.0
h = 50.0

y = 0.0
while y < h:
	x = 0.0
	while x < w:
		Zr, Zi, Tr, Ti = 0.0, 0.0, 0.0, 0.0
		Cr = 2*x/w - 1.5
		Ci = 2*y/h - 1.0

		i = 0
		while i < 50 and Tr+Ti <= 4:
			Zi = 2*Zr*Zi + Ci
			Zr = Tr - Ti + Cr
			Tr = Zr * Zr
			Ti = Zi * Zi
			i = i+1

		if Tr+Ti <= 4:
			# print('*', end='')
			pass
		else:
			# print('·', end='')
			pass

		x = x+1

	# print()
	y = y+1
