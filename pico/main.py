import time

try:
    import ppwhttp
except ImportError:
    raise RuntimeError("Cannot find ppwhttp. Have you copied ppwhttp.py to your Pico?")

import time
from machine import Pin, PWM

brightness = 0.125

@ppwhttp.route("/", methods=["GET", "POST"])
def get_home(method, url, data=None):
    global brightness
    
    if method == "POST":        
        brightness = int(data.get("brightness", 1))
        print("Set DIV to {}".format(brightness))

    return """<form method="post" action="/">
    <input name="brightness" type="number" value="{brightness}"  />
    <input type="submit" value="Set brightness" />
</form>""".format(brightness=brightness)


# how to use:
# curl -X POST 192.168.178.34/set_brightness -H "Content-Type: application/x-www-form-urlencoded" -d "brightness=0.5&version=1.0"
@ppwhttp.route("/set_brightness", methods=["POST"])
def post_set_dimmer(method, url, data=None):
    
    global brightness
    
    try:
        brightness = float(data['brightness'])
    except Exception as e:
        print(f"Error reading 'brightness' value: {e}")
        
    return f"/set_brightness received: {data}\n"


def setup_server():
    ppwhttp.start_wifi()

    my_ip = ppwhttp.get_ip_address()
    print("Local IP: {}.{}.{}.{}".format(*my_ip))

    return ppwhttp.start_server()


def setup_pwm():
    pwm = PWM(Pin(14))
    pwm.freq(5000)
    
    return pwm


def clip_u16(value):
    return max(min(int(value), 255**2), 0)


if __name__ == "__main__":
    server_sock = setup_server()        
    pwm = setup_pwm()

    while True:
        ppwhttp.handle_http_request(server_sock)
        pwm.duty_u16(clip_u16(255**2 * brightness))
        time.sleep(0.1)

