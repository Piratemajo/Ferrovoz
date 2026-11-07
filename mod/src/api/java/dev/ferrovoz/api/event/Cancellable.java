package dev.ferrovoz.api.event;

/**
 * An event fired before something happens, which can be prevented. These are only
 * fired when a player asks for it (with {@code /voice}); what a plugin does through
 * the API never fires them, so two plugins can't fight each other or loop.
 *
 * <p>When cancelled, Ferrovoz tells the player nothing: the plugin that cancels it
 * should say why.
 */
public interface Cancellable extends VoiceEvent {
    boolean isCancelled();

    void setCancelled(boolean cancelled);
}
