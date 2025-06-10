#include <gst/gst.h>
#include <gst/rtp/rtp.h>
#include <gst/rtp/gstrtpbuffer.h>
#include <signal.h>
#include <string.h>
#include <stdbool.h>
#include <stdlib.h>

// Constants
#define REMOTE_HOST "192.168.1.7"
#define REMOTE_RTP_PORT 5002
#define REMOTE_RTCP_PORT 5001
#define LOCAL_RTCP_PORT 5003
#define VIDEO_SSRC 2222

// Global variables
static GstElement* pipeline = NULL;
static GMainLoop* main_loop = NULL;
static guint bus_watch_id = 0;

// Function declarations
static void create_gstreamer_pipeline(void);
static void cleanup(void);
static void signal_handler(int signum);
static gboolean bus_call(GstBus* bus, GstMessage* msg, gpointer data);
static gboolean handle_qos(GstBus* bus, GstMessage* msg, gpointer data);
static GstPadProbeReturn caps_probe_cb(GstPad* pad, GstPadProbeInfo* info, gpointer user_data);
static gboolean monitor_pipeline(gpointer data);

static GstPadProbeReturn caps_probe_cb(GstPad* pad, GstPadProbeInfo* info, gpointer user_data) {
    GstEvent *event = GST_PAD_PROBE_INFO_EVENT(info);
    
    if (GST_EVENT_TYPE(event) == GST_EVENT_CAPS) {
        GstCaps* caps;
        gst_event_parse_caps(event, &caps);
        gchar *caps_str = gst_caps_to_string(caps);
        g_print("Caps event at src: %s\n", caps_str);
        g_free(caps_str);
    }
    
    return GST_PAD_PROBE_OK;
}

static void create_gstreamer_pipeline(void) {
    // Create a pipeline
    pipeline = gst_pipeline_new("streaming-pipeline");
    GstElement* rtpbin = gst_element_factory_make("rtpbin", "rtpbin");
    GstElement* bundle = gst_element_factory_make("rtpfunnel", "bundle");
    GstElement* rtcp_bundler = gst_element_factory_make("funnel", "rtcp_bundler");
    
    // Create video elements
    GstElement* src = gst_element_factory_make("d3d12screencapturesrc", "d3d12screencapturesrc");
    GstElement* capsfilter = gst_element_factory_make("capsfilter", "capsfilter");
    GstElement* d3d12download = gst_element_factory_make("d3d12download", "d3d12download");
    GstElement* videoconvert = gst_element_factory_make("videoconvert", "videoconvert");
    GstElement* videoscale = gst_element_factory_make("videoscale", "videoscale");
    GstElement* x264enc = gst_element_factory_make("x264enc", "x264enc");
    GstElement* rtph264pay = gst_element_factory_make("rtph264pay", "rtph264pay");

    // Create network elements
    GstElement* rtp_sink = gst_element_factory_make("udpsink", "rtp_sink");
    GstElement* rtcp_sink = gst_element_factory_make("udpsink", "rtcp_sink");
    GstElement* rtcp_src = gst_element_factory_make("udpsrc", "rtcp_src");

    GstElement* queue = gst_element_factory_make("queue", "queue");
    GstElement* queue2 = gst_element_factory_make("queue", "queue2");

    // Check elements
    if (!pipeline || !rtpbin || !bundle || !rtcp_bundler || !src || !videoconvert ||
        !videoscale || !capsfilter || !x264enc || !rtph264pay || !rtp_sink || !rtcp_sink || !rtcp_src) {
        g_printerr("One or more elements could not be created\n");
        cleanup();
        return;
    }

    // Configure final output caps (after videoconvert)
    GstCaps* caps = gst_caps_new_simple("video/x-raw",
                                        "format", G_TYPE_STRING, "I420", // Explicitly specify I420
                                        "width", G_TYPE_INT, 1280,
                                        "height", G_TYPE_INT, 720,
                                        "framerate", GST_TYPE_FRACTION, 60, 1,
                                        NULL);
    g_object_set(capsfilter, "caps", caps, NULL);
    gst_caps_unref(caps);

    g_object_set(rtpbin, 
        "do-retransmission", FALSE,
        "rtp-profile", GST_RTP_PROFILE_AVPF,
        NULL);

    g_object_set(src,
        "show-cursor", TRUE, 
        "monitor-index", 0,
        NULL);

    g_object_set(x264enc,
        "bitrate", 6000,
        "speed-preset", 1,
        "key-int-max", 120,
        "tune", 0x00000004,
        "b-adapt", FALSE,
        "bframes", 0,
        "pass", 0,
        NULL);

    g_object_set(rtph264pay, 
        "ssrc", VIDEO_SSRC,
        "pt", 100,
        "config-interval", -1,
        NULL);

    g_object_set(rtp_sink,
        "host", REMOTE_HOST,
        "port", REMOTE_RTP_PORT,
        "sync", FALSE,
        "async", FALSE,
        NULL);
    g_object_set(rtcp_sink, 
        "host", REMOTE_HOST,
        "port", REMOTE_RTCP_PORT,
        "sync", FALSE,
        "async", FALSE,
        "buffer-size", 4194304,
        NULL);
    g_object_set(rtcp_src,
        "port", LOCAL_RTCP_PORT,
        "buffer-size", 1048576,
        NULL);

    // Add elements to pipeline
    gst_bin_add_many(GST_BIN(pipeline), rtpbin, bundle, rtcp_bundler,
                     src, videoconvert, videoscale, d3d12download,
                     x264enc, rtph264pay, queue, queue2,
                     rtp_sink, rtcp_sink, rtcp_src, capsfilter, NULL);

    // Link elements
    if (!gst_element_link_many(src, d3d12download, videoconvert, videoscale, capsfilter, queue2, x264enc, queue, rtph264pay, NULL)) {
        g_printerr("Failed to link elements\n");
        cleanup();
        return;
    }

    // Link RTP elements
    if (!gst_element_link_pads(rtph264pay, "src", rtpbin, "send_rtp_sink_1") ||
        !gst_element_link_pads(rtpbin, "send_rtp_src_1", bundle, "sink_0") ||
        !gst_element_link_pads(rtpbin, "send_rtcp_src_1", rtcp_bundler, "sink_0") ||
        !gst_element_link(bundle, rtp_sink) ||
        !gst_element_link(rtcp_bundler, rtcp_sink) ||
        !gst_element_link_pads(rtcp_src, "src", rtpbin, "recv_rtcp_sink_1")
    ) {
        g_printerr("Failed to link RTP elements\n");
        cleanup();
        return;
    }

    gchar *dot = gst_debug_bin_to_dot_data(GST_BIN(pipeline), GST_DEBUG_GRAPH_SHOW_ALL);
    g_file_set_contents("D:/gstreamer_dumps/manual_pipeline12.dot", dot, -1, NULL);
    g_free(dot);

    // Add caps probe for debugging
    GstPad* src_pad = gst_element_get_static_pad(src, "src");
    if (src_pad) {
        gst_pad_add_probe(src_pad, GST_PAD_PROBE_TYPE_EVENT_DOWNSTREAM, caps_probe_cb, NULL, NULL);
        gst_object_unref(src_pad);
    }

    GstBus* bus = gst_pipeline_get_bus(GST_PIPELINE(pipeline));

    if (bus_watch_id > 0) {
        gst_bus_remove_watch(bus);
        bus_watch_id = 0;
    }
    
    bus_watch_id = gst_bus_add_watch(bus, bus_call, NULL);

    g_signal_connect(bus, "message::qos", G_CALLBACK(handle_qos), NULL);
    
    gst_object_unref(bus);

    // Start playing
    GstStateChangeReturn ret = gst_element_set_state(pipeline, GST_STATE_PLAYING);
    if (ret == GST_STATE_CHANGE_FAILURE) {
        g_printerr("Failed to start pipeline\n");
        cleanup();
        return;
    } else if (ret == GST_STATE_CHANGE_ASYNC) {
        g_print("Pipeline is changing state asynchronously...\n");
    } else {
        g_print("Pipeline state change successful\n");
    }

    GstState state;
    ret = gst_element_get_state(pipeline, &state, NULL, GST_CLOCK_TIME_NONE);
    g_print("Pipeline current state: %s\n", gst_element_state_get_name(state));

    g_print("GStreamer pipeline started\n");
}

static gboolean handle_qos(GstBus* bus, GstMessage* msg, gpointer data) {
    if (GST_MESSAGE_TYPE(msg) == GST_MESSAGE_QOS) {
        GstFormat format;
        guint64 processed, dropped;
        gst_message_parse_qos_stats(msg, &format, &processed, &dropped);
        g_print("QoS: processed=%" G_GUINT64_FORMAT ", dropped=%" G_GUINT64_FORMAT "\n",
                processed, dropped);
    }
    return TRUE;
}

static gboolean bus_call(GstBus* bus, GstMessage* msg, gpointer data) {
    switch (GST_MESSAGE_TYPE(msg)) {
        case GST_MESSAGE_ERROR: {
            GError* err;
            gchar* debug;
            gst_message_parse_error(msg, &err, &debug);
            g_print("Error: %s\n", err->message);
            g_print("Debug info: %s\n", debug ? debug : "none");
            g_error_free(err);
            g_free(debug);
            g_main_loop_quit(main_loop);
            break;
        }
        case GST_MESSAGE_EOS:
            g_print("End of stream\n");
            g_main_loop_quit(main_loop);
            break;
        case GST_MESSAGE_STATE_CHANGED: {
            GstState old_state, new_state;
            gst_message_parse_state_changed(msg, &old_state, &new_state, NULL);
            g_print("Element %s changed state from %s to %s\n",
                GST_OBJECT_NAME(msg->src),
                gst_element_state_get_name(old_state),
                gst_element_state_get_name(new_state));
            break;
        }
        default:
            break;
    }
    return TRUE;
}

static void cleanup(void) {
     if (pipeline) {
        GstBus* bus = gst_pipeline_get_bus(GST_PIPELINE(pipeline));
        if (bus_watch_id > 0) {
            gst_bus_remove_watch(bus);
            bus_watch_id = 0;
        }
        gst_object_unref(bus);
        gst_element_set_state(pipeline, GST_STATE_NULL);
        gst_object_unref(pipeline);
        pipeline = NULL;
    }
    if (main_loop) {
        g_main_loop_quit(main_loop);
        g_main_loop_unref(main_loop);
        main_loop = NULL;
    }
}

static void signal_handler(int signum) {
    cleanup();
}

static gboolean monitor_pipeline(gpointer data) {
    GstElement* pipeline = GST_ELEMENT(data);
    GstQuery* query = gst_query_new_latency();

    if (gst_element_query(pipeline, query)) {
        gboolean live;
        GstClockTime min_latency, max_latency;
        gst_query_parse_latency(query, &live, &min_latency, &max_latency);
        g_print("Latency - Min: %" GST_TIME_FORMAT " Max: %" GST_TIME_FORMAT "\n",
                GST_TIME_ARGS(min_latency), GST_TIME_ARGS(max_latency));
    }

    // Monitor queue levels in the pipeline
    GstElement* x264enc = gst_bin_get_by_name(GST_BIN(pipeline), "x264enc");

    return G_SOURCE_CONTINUE;
}

int main(int argc, char* argv[]) {
    gst_init(&argc, &argv);
    signal(SIGINT, signal_handler);

    // Create and start the pipeline
    create_gstreamer_pipeline();

    g_timeout_add_seconds(1, monitor_pipeline, pipeline);

    // Main loop
    main_loop = g_main_loop_new(NULL, FALSE);
    g_print("Entering main loop...\n");
    g_main_loop_run(main_loop);

    cleanup();
    return 0;
}
