from browser import request_animation_frame

w = 50.0
h = 50.0

# to make up for the lack of `global`
_y = {'y': 0.0}

def mandel(_time_elapsed=None):
    y = _y['y']
    if y >= h:
        return
    x = 0.0
    while x < w:
        Zr, Zi, Tr, Ti = 0.0, 0.0, 0.0, 0.0
        Cr = 2 * x / w - 1.5
        Ci = 2 * y / h - 1.0

        i = 0
        while i < 50 and Tr + Ti <= 4:
            Zi = 2 * Zr * Zi + Ci
            Zr = Tr - Ti + Cr
            Tr = Zr * Zr
            Ti = Zi * Zi
            i += 1

        if Tr + Ti <= 4:
            print('*', end='')
        else:
            print('·', end='')

        x += 1

    print()
    _y['y'] += 1
    request_animation_frame(mandel)

request_animation_frame(mandel)
