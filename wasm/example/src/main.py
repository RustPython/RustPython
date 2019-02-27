from browser import fetch

def fetch_handler(repos):
    star_sum = 0
    for repo in repos:
        star_sum += repo['stars']
    print(f'Average github trending star count: {star_sum / len(repos)}')


fetch(
    'https://github-trending-api.now.sh/repositories',
    response_format='json',
).then(fetch_handler)