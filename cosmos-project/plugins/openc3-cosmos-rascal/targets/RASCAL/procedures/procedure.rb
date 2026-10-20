# Script Runner test script
cmd("RASCAL EXAMPLE")
wait_check("RASCAL STATUS BOOL == 'FALSE'", 5)
