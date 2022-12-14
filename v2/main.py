import network
import time

from machine import Pin
import uasyncio as asyncio


"""    
curl -X POST 192.168.178.36/set_brightness/brightness=<BRIGHTNESS_VALUE>

with 0.0 <= BRIGHTNESS_VALUE <= 1.0
"""

WIFI_SSID = "FRITZ!Box 6690 NW"
WIFI_PASS = "57433726856370259000"

brightness = 0.125

onboard = Pin("LED", Pin.OUT, value=0)

wlan = network.WLAN(network.STA_IF)

html = """<!DOCTYPE html>
15 <html>
16 <head> <title>Pico W</title> </head>
17 <body> <h1>Pico W</h1>
18 <p>brightness set to %s</p>
19 </body>
20 </html>
21 """

pwm = None

def clip_u16(value):
    return max(min(int(value), 255**2), 0)


def setup_pwm():
    global brightness
    global pwm
    pwm = PWM(Pin(14))
    pwm.freq(5000)
    time.sleep(0)
    pwm.duty_u16(clip_u16(255**2 * brightness))  
    

def parse_request(request):
    return request.split('set_brightness/brightness=')[-1].split(" ")[0]
    

def connect_to_network():
    wlan.active(True)
    wlan.config(pm = 0xa11140) # Disable power-save mode
    wlan.connect(WIFI_SSID, WIFI_PASS)

    max_wait = 10
    while max_wait > 0:
        if wlan.status() < 0 or wlan.status() >= 3:
            break
        max_wait -= 1
        print('waiting for connection...')
        time.sleep(1)

    if wlan.status() != 3:
        raise RuntimeError('network connection failed')

    print('connected')
    status = wlan.ifconfig()
    print('ip = ' + status[0])


async def serve_client(reader, writer):
    global brightness
    global pwm
    
    print("Client connected")
    request_line = await reader.readline()
    print("Request:", request_line)
    
    #  We are not interested in HTTP request headers, skip them
    while await reader.readline() != b"\r\n":
        pass
    
    request = str(request_line) 
   
    try:
        brightness = float(str(parse_request(request)))
        pwm.duty_u16(clip_u16(255**2 * brightness))     
        response = html % str(brightness)
    
    except Exception as e:
        print(f"Error reading 'brightness' value: {e}")        
        response = html % f"Error setting {brightness}"
    
    writer.write('HTTP/1.0 200 OK\r\nContent-type: text/html\r\n\r\n')
    writer.write(response)

    await writer.drain()
    await writer.wait_closed()
    print("Client disconnected")

async def main():
    print('Connecting to Network...')
    connect_to_network()
    
    print('Setup PWM')
    setup_pwm()

    print('Setting up webserver...')
    asyncio.create_task(asyncio.start_server(serve_client, "0.0.0.0", 80))
    
    while True:
        onboard.on()
        print("heartbeat")
        await asyncio.sleep(0.25)
        onboard.off()       
        await asyncio.sleep(5)

try:
    asyncio.run(main())
finally:
    asyncio.new_event_loop()


