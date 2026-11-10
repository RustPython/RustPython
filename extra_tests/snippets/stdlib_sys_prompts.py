import sys

# Importing sys in a non-interactive program must not create REPL prompts.
# Their absence also lets inline pdb distinguish scripts from a REPL session.
assert not hasattr(sys, "ps1")
assert not hasattr(sys, "ps2")
