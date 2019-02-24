for i in range(1, 100):
    print(f"{i} ", end="")
    if not i % 3:
        print("fizz", end="")
    if not i % 5:
        print("buzz", end="")
    print()
