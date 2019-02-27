from browser import fetch

def fetch_handler(res):
    print(f"headers: {res['headers']}")

fetch(
    "https://httpbin.org/get",
    response_format="json",
    headers={
        "X-Header-Thing": "rustpython is neat!"
    },
).then(fetch_handler, lambda err: print(f"error: {err}"))
