# Dimmer for LED Stripes
This will set up a simple REST service on your Pi Pico that allows to set the brightness value remotely. Internally, 
the PWM output will be scaled by the brightness value. Note that, the brightness should be in the range from 0.0 to 1.0.

There are two version of this project:
1. The first version requires the [Pimoroni Wireless Pack](https://shop.pimoroni.com/products/pico-wireless-pack?variant=32369508581459). This is because originally the Pi Pico was not equipped with a Wifi module.
2. The second version requires a Pi Pico with a Wifi module. 

## Version 1: Pi Pico with [Pimoroni Wireless Pack](https://shop.pimoroni.com/products/pico-wireless-pack?variant=32369508581459)

### Hardware requirements
A Raspberry Pi Pico is required which needs to be attached to a [Pimoroni Wireless Pack](https://shop.pimoroni.com/products/pico-wireless-pack?variant=32369508581459).

Further, I use the PWM of GPIO Pin 14 and connect it to an [amplifier](https://www.amazon.de/gp/product/B07VRCXGFY/ref=ppx_yo_dt_b_asin_title_o03_s00?ie=UTF8&psc=1). 
Make sure to connect GND of GPIO Pin 14 (Pin 18 on the Pi) to GND of the amplifier. 
Obviously, GPIO Pin 14 (Pin 19 on the Pi) needs to be connected to V+ of the amplifier.
The LED stripes need to be connected to the amplifier (power supply to input, LED stripes to the output).

I used these [LED stripes](https://www.amazon.de/Pflanzenlampe-Samsung%EF%BC%86Full-Helligkeitsstufe-pflanzenlicht-Zimmerpflanzen/dp/B088QVN89Q/ref=sr_1_5?crid=2IQNDREOYV7LG&keywords=led+pflanzenlampe&qid=1657228824&sprefix=led+pfl%2Caps%2C148&sr=8-5).
The power supply output is 24V. 

### Setup
1. Install [pimoroni-pico-v1.18.7-micropython.uf2](https://github.com/pimoroni/pimoroni-pico/releases/tag/v1.18.7) on your Pi Pico.
2. Save [ppwhttp.py](https://github.com/pimoroni/pimoroni-pico/tree/main/micropython/examples/pico_wireless) on your Pi Pico. 
3. Adapt ```WIFI_SSID``` and ```WIFI_PASS``` in secrets.py and also upload this file to your Pi Pico.

Note: As of today v1.19.x is already available. However, that version contains breaking changes and the examples including the rest server haven't been updated yet.

## Version 2: Pi Pico with Wifi onboard

### Hardware requirements
Since the Wifi module is integrated on the Pi Pico, there is no need to wire an additional board. Still, the amplifier must be connected as described in the section above.

## Usage
### Option 1: Browser-based
Simply enter the IP address of the Pico in your web browser. A simple website should appear that allows you
to enter a brightness value. As soon as you click on enter you should see the effect on your LED stripes.

### Option 2: REST call
Simply use e.g. curl on the terminal to send a REST request. Replace ```<PICO_IP_ADDRESS>``` by the Pico's IP address and ```<BRIGHTNESS_VALUE>``` by a value between 0.0 and 1.0.

```bash
curl -X POST <PICO_IP_ADDRESS>/set_brightness \ 
     -H "Content-Type: application/x-www-form-urlencoded" \ 
     -d "brightness=<BRIGHTNESS_VALUE>"
``` 

## Application
A flutter-based client app that is able to control the light dimmer can be found here: [https://github.com/jrhahn/flutter_light_control](https://github.com/jrhahn/flutter_light_control)
