# unittest for modified imghdr.py
# Should be replace it into https://github.com/python/cpython/blob/master/Lib/test/test_imghdr.py
import os
import imghdr


TEST_FILES = (
    #('python.png', 'png'),
    ('python.gif', 'gif'),
    ('python.bmp', 'bmp'),
    ('python.ppm', 'ppm'),
    ('python.pgm', 'pgm'),
    ('python.pbm', 'pbm'),
    ('python.jpg', 'jpeg'),
    ('python.ras', 'rast'),
    #('python.sgi', 'rgb'),
    ('python.tiff', 'tiff'),
    ('python.xbm', 'xbm'),
    ('python.webp', 'webp'),
    ('python.exr', 'exr'),
)

resource_dir = os.path.join(os.path.dirname(__file__), 'imghdrdata')

for fname, expected in TEST_FILES:
    res = imghdr.what(os.path.join(resource_dir, fname))
    assert res == expected