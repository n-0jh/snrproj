# snrproj enviornment

## current structure
* **'cosmos-project/** - OpenC3 core
* **mock_lora_forc3/** - A mock rust application to interpret data on C3

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
 
