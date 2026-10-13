<?php
function get_debug_type(mixed $value) {
    if ($value === null) { return "null"; }
    if (is_bool($value)) { return "bool"; }
    if (is_int($value)) { return "int"; }
    if (is_float($value)) { return "float"; }
    if (is_string($value)) { return "string"; }
    if (is_array($value)) { return "array"; }
    if (is_object($value)) {
        $name = get_class($value);
        $at = strpos($name, "@anonymous");
        if ($at !== false) {
            return substr($name, 0, $at + 10);
        }
        return $name;
    }
    if (is_resource($value)) {
        $t = get_resource_type($value);
        if ($t === "Unknown") { return "resource (closed)"; }
        return "resource (" . $t . ")";
    }
    return "mixed";
}
