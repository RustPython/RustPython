def test_super_list():
    test_super_list = super(list)
    assert test_super_list.__self__ is None
    assert test_super_list.__self_class__ is None
    assert test_super_list.__thisclass__ == list
