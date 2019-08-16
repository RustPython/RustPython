# unittest for modified imghdr.py
# Should be replace it into https://github.com/python/cpython/blob/master/Lib/test/test_imghdr.py

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

for fname, expected in TEST_FILES:
    res = imghdr.what('tests/snippets/imghdrdata/'+fname)
    assert res == expected