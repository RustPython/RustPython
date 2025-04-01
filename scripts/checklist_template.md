{% macro display_line(i) %}- {% if i.completed == True %}[x] {% elif i.completed == False %}[ ] {% endif %}{{ i.name }}{% if i.pr != None %} {{ i.pr }}{% endif %}{% endmacro %}
# List of libraries

{% for lib in update_libs %}{{ display_line(lib) }}
{% endfor %}

# List of un-added libraries
These libraries are not added yet. Pure python one will be possible while others are not.

{% for lib in add_libs %}{{ display_line(lib) }}
{% endfor %}

# List of tests without python libraries

{% for lib in update_tests %}{{ display_line(lib) }}
{% endfor %}

# List of un-added tests without python libraries

{% for lib in add_tests %}{{ display_line(lib) }}
{% endfor %}
