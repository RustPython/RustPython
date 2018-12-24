# WARNING! This file contains mixed tabs and spaces
# (because that's what it is testing)

def weird_indentation():
	    return_value = "hi"
	    if False:
		    return return_value
	    return "hi"

assert weird_indentation() == "hi"
   
