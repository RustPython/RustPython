for i in range(12):
    print('{:d}, '.format(chr(i) in "ss"), end='')
    if i % 16 == 1:
        print()