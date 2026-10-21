# snrproj enviornment

## current structure
* **'cosmos-project/** - OpenC3 core
* **cosmos-project/plugins/openc3-cosmos-rascal/** - plugin that defines the RASCAL target and telemetry packet
* **mock_lora_forc3/** - A mock rust 
application to interpret data on C3

## prereq
install the following
* **docker desktop** 
* **Rust**

## for installation
(for security local .env are ignored)
so to install openc3

1. navigate to:
``` cd cosmos-project ```

2. create local env from the template I have included
``` cp .env.example .env ```

3. launch openc3
* **linux/mac** 
``` ./openc3.sh run ```
* **windows**
``` openc3.bat run ```

4. once ran, visit:
**`http://localhost:2900`**

## build and install RASCAL plugin
OpenC3 does not, by default, know our telemetry packet is there, but it will once the plugin is installed.

1. build it
from (cosmos-project/plugins/openc3-cosmos-rascal):

* **linux/mac**
```bash
cd plugins/openc3-cosmos-rascal
../../openc3.sh cli rake build VERSION=1.0.0
```
* **windows**
```bat
cd plugins\openc3-cosmos-rascal
..\..\openc3.bat cli rake build VERSION=1.0.0
```
This creates `openc3-cosmos-rascal-1.0.0.gem` in that folder.
 
NOTE: Upon new VERSION1.x.x, click "UPDATE" in OpenC3 under Admin>Installed plugins.

2. in OpenC3 go to Admin > Plugins > Install from file choose `.gem`, check `rascal_tlm_port`is **6900** and `rascal_cmd_port` is **6901** then click **Install**

3. Rascal should now be viewable in the Installed Plugins list

4. run telemetry from command line
```bash
cd mock_lora_forc3
cargo run --release
```
counter should be incrementing once per second

## view the telemetry
1. open **Packet Viewer** from the OpenC3 tool menu
2. select target **RASCAL**, packet **HEALTH**
3. `COUNTER` should match that in your command line

to check it is arriving, open **CmdTlmServer**:
* **Interfaces** tab: `RASCAL_INT` should be CONNECTED and its packet count should go up
* **Packets** tab: `RASCAL HEALTH` received count should go up

## packet format
big-endian, once per second.
NOTE: must match what is inside of 'mock_lore_forc3/src/main.rs` and `cosmos-project/plugins/openc3-cosmos-rascal-targets/RASCAL/cmd_tlm/tlm.txt`

## ports
port | use 
2900 | OpenC3 web UI
6900/udp | telemetry into OpenC3
6901/udp | commands out of OpenC3 
7799-7814/udp | reserved by OpenC3