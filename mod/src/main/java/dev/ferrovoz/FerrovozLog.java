package dev.ferrovoz;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Logger compartido. Va aparte de {@link Ferrovoz} porque esa clase implementa
 * una interfaz de Fabric, y cargarla en Paper/Spigot (donde Fabric no existe)
 * fallaría. SLF4J está tanto en Fabric como en Paper.
 */
public final class FerrovozLog {
    public static final Logger LOG = LoggerFactory.getLogger("ferrovoz");

    private FerrovozLog() {}
}
