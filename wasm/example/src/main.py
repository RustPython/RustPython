from browser import fetch, alert

def fetch_handler(repos):
    star_sum = 0
    for repo in repos:
        star_sum += repo['stars']
    alert(f'Average github trending star count: {star_sum / len(repos)}')

fetch(
    'https://github-trending-api.now.sh/repositories',
    response_format='json',
).then(fetch_handler, lambda err: alert(f"Error: {err}"))