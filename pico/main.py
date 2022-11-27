try:
    import ppwhttp
except ImportError:
    raise RuntimeError("Cannot find ppwhttp. Have you copied ppwhttp.py to your Pico?")

import picowireless
import time

from machine import Pin, PWM
from secrets import WIFI_SSID, WIFI_PASS

__VERSION = "0.2.0"
__SOURCE_URL = "https://github.com/jrhahn/pico_pi_dimmer"

brightness = 0.125

@ppwhttp.route("/", methods=["GET", "POST"])
def get_home(method, url, data=None):
    global __VERSION
    global __SOURCE_URL
    return f'<p style="font-family:courier;">' \
            + f'This is PyDimmer. <br /><br />' \
            + f'source: <a href={__SOURCE_URL}>{__SOURCE_URL}</a><br />' \
            + f'version: {__VERSION}' \
            + f'</p>'

# Test via curl:
#      curl -X POST 192.168.178.34/set_brightness -H "Content-Type: application/x-www-form-urlencoded" -d "brightness=0.5"
@ppwhttp.route("/set_brightness", methods=["POST"])
def endpoint_set_brightness(method, url, data=None):    
    global brightness
    
    try:
        brightness = float(data['brightness'])
    except Exception as e:
        print(f"Error reading 'brightness' value: {e}")
        
    return f"brightness set to: {data}\n"

# Test via curl:
#      curl -X GET 192.168.178.34/get_brightness 
@ppwhttp.route("/get_brightness", methods=["GET"])
def endpoint_get_brightness(methods=None, url=None, data=None):
    global brightness
    
    return f'{{ "brightness": {brightness} }}'


def start_wifi(wifi_ssid=WIFI_SSID, wifi_pass=WIFI_PASS):
    if wifi_ssid is None or wifi_pass is None:
        return "WiFi SSID/PASS required. Set them in secrets.py and" \
                + " copy it to your Pico, or pass them as arguments."
    
    wait_duration_ms = 30000
    
    while True:
        picowireless.init()

        print("Connecting to {}...".format(wifi_ssid))
        picowireless.wifi_set_passphrase(wifi_ssid, wifi_pass)
        
        timer_start = time.ticks_ms()
        
        while time.ticks_ms() <= timer_start + wait_duration_ms:
            if picowireless.get_connection_status() == 3:
                return "Connected."

        print("Connecting failed. Restarting Wifi...")


def setup_server():
    print(start_wifi())

    ip_address = ppwhttp.get_ip_address()
    print("Local IP: {}.{}.{}.{}".format(*ip_address))

    return ppwhttp.start_server()


def setup_pwm():
    pwm = PWM(Pin(14))
    pwm.freq(5000)
    
    return pwm


def clip_u16(value):
    return max(min(int(value), 255**2), 0)


if __name__ == "__main__":
    server_socket = setup_server()        
    pwm = setup_pwm()

    while True:
        ppwhttp.handle_http_request(server_socket)
        pwm.duty_u16(clip_u16(255**2 * brightness))
