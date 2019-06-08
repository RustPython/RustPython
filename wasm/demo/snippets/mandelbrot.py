w = 50.0
h = 50.0

def mandel():
    """Print a mandelbrot fractal to the console, yielding after each character is printed"""
    y = 0.0
    while y < h:
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
            yield

        print()
        y += 1
        yield

# run the mandelbrot

try: from browser import request_animation_frame
except: request_animation_frame = None

gen = mandel()
def gen_cb(_time=None):
    for _ in range(4): gen.__next__()
    request_animation_frame(gen_cb)
if request_animation_frame: gen_cb()
else: any(gen)
