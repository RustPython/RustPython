import io
import csv

text = '''rust,https://rust-lang.org
python,https://python.org
'''

with io.StringIO(text) as file:
	reader = csv.reader(file)

	it = iter(reader)

	[lang, addr] = next(it)
	assert lang == "rust"
	assert addr == "https://rust-lang.org"

	[lang, addr] = next(it)
	assert lang == "python"
	assert addr == "https://python.org"
