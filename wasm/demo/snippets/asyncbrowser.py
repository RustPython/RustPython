import browser
import asyncweb

async def main(delay):
    url = f"https://httpbin.org/delay/{delay}"
    print(f"fetching {url}...")
    res = await browser.fetch(
        url, response_format="json", headers={"X-Header-Thing": "rustpython is neat!"}
    )
    print(f"got res from {res['url']}:")
    print(res, end="\n\n")


for delay in range(3):
    asyncweb.run(main(delay))
print()
