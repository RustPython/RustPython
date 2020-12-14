assert 'spam eggs' == 'spam eggs'  # single quotes
assert "doesn't" == 'doesn\'t'  # use \' to escape the single quote...
assert "doesn't" == "doesn't"  # ...or use double quotes instead
assert '"Yes," he said.' == '"Yes," he said.'
assert '"Yes," he said.' == "\"Yes,\" he said."
assert '"Isn\'t," she said.' == '"Isn\'t," she said.'

