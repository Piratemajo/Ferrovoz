package dev.ferrovoz.bukkit;

import dev.ferrovoz.CommandRedaction;
import org.apache.logging.log4j.Level;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Marker;
import org.apache.logging.log4j.core.LogEvent;
import org.apache.logging.log4j.core.Logger;
import org.apache.logging.log4j.core.filter.AbstractFilter;
import org.apache.logging.log4j.message.Message;

/**
 * Filtro de log4j en el logger raíz de Paper: descarta la línea "issued server command"
 * de {@code /voz grupo crear|unirse} con contraseña y escribe en su lugar la misma con
 * la contraseña tapada ({@link CommandRedaction}). Es lo que hacen los plugins de login.
 * Las demás líneas pasan sin tocar.
 */
final class PasswordLogFilter extends AbstractFilter {
    private final java.util.logging.Logger out;
    private volatile boolean active = true;

    private PasswordLogFilter(java.util.logging.Logger out) {
        this.out = out;
    }

    /** Lo engancha al logger raíz. Lanza si el servidor no usa log4j (no pasa en Paper ni Spigot). */
    static PasswordLogFilter install(java.util.logging.Logger out) {
        PasswordLogFilter filter = new PasswordLogFilter(out);
        filter.start();
        ((Logger) LogManager.getRootLogger()).addFilter(filter);
        return filter;
    }

    /** log4j no deja quitarlo con seguridad: se queda, pero deja pasar todo. */
    void deactivate() {
        active = false;
    }

    private Result check(String line) {
        if (!active) return Result.NEUTRAL;
        String redacted = CommandRedaction.redact(line);
        if (redacted == null) return Result.NEUTRAL;
        out.info(redacted);
        return Result.DENY;
    }

    @Override
    public Result filter(LogEvent event) {
        Message m = event.getMessage();
        return check(m == null ? null : m.getFormattedMessage());
    }

    @Override
    public Result filter(Logger logger, Level level, Marker marker, String msg, Object... params) {
        return check(msg);
    }

    @Override
    public Result filter(Logger logger, Level level, Marker marker, Object msg, Throwable t) {
        return check(msg == null ? null : msg.toString());
    }

    @Override
    public Result filter(Logger logger, Level level, Marker marker, Message msg, Throwable t) {
        return check(msg == null ? null : msg.getFormattedMessage());
    }
}
