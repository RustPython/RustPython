import threading
import time

def runner():
    print('runner done')

threading.Thread(target=runner).start()
time.sleep(1)
print('main done')
