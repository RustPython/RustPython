repos = js_vars['repos']

star_sum = 0

for repo in repos:
    star_sum += repo['stars']

return 'Average github trending star count: ' + str(star_sum / len(repos))
