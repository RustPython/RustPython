try:
    try:
        raise ValueError()
    except ValueError as e:
        raise RuntimeError() from e
except RuntimeError as e:
    pass
