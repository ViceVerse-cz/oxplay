/* SPDX-License-Identifier: GPL-3.0-or-later
 * Decode the bundled synthetic AV1 motion clip with explicit CPU libdav1d.
 * No window, audio output, hardware device, or rendering API is involved.
 */
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/error.h>
#include <libavutil/pixfmt.h>

static int receive_frames(AVCodecContext *decoder, AVFrame *frame,
                          uint64_t hashes[8], int *count)
{
    int result;
    while ((result = avcodec_receive_frame(decoder, frame)) >= 0) {
        if (*count >= 8 || frame->width != 64 || frame->height != 64 ||
            frame->format != AV_PIX_FMT_YUV420P || !frame->data[0] ||
            decoder->hw_device_ctx || decoder->hw_frames_ctx) {
            fprintf(stderr, "Unexpected software AV1 frame/dimensions/count\n");
            return AVERROR_INVALIDDATA;
        }
        uint64_t hash = UINT64_C(14695981039346656037);
        for (int y = 0; y < frame->height; ++y) {
            const uint8_t *row = frame->data[0] + y * frame->linesize[0];
            for (int x = 0; x < frame->width; ++x)
                hash = (hash ^ row[x]) * UINT64_C(1099511628211);
        }
        hashes[(*count)++] = hash;
        av_frame_unref(frame);
    }
    return result == AVERROR(EAGAIN) || result == AVERROR_EOF ? 0 : result;
}

int main(int argc, char **argv)
{
    AVFormatContext *input = NULL;
    AVCodecContext *decoder = NULL;
    AVPacket *packet = NULL;
    AVFrame *frame = NULL;
    uint64_t hashes[8] = {0};
    int count = 0, packets = 0, result = AVERROR_INVALIDDATA, stream;
    const AVCodec *codec = avcodec_find_decoder_by_name("libdav1d");
    if (argc != 2 || !codec || codec->id != AV_CODEC_ID_AV1) {
        fprintf(stderr, "Required software libdav1d AV1 decoder is missing\n");
        goto done;
    }
    if ((result = avformat_open_input(&input, argv[1], NULL, NULL)) < 0 ||
        (result = avformat_find_stream_info(input, NULL)) < 0)
        goto done;
    stream = av_find_best_stream(input, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
    if (stream < 0) { result = stream; goto done; }
    if (input->streams[stream]->codecpar->codec_id != AV_CODEC_ID_AV1) {
        result = AVERROR_INVALIDDATA;
        goto done;
    }
    decoder = avcodec_alloc_context3(codec);
    packet = av_packet_alloc();
    frame = av_frame_alloc();
    if (!decoder || !packet || !frame) { result = AVERROR(ENOMEM); goto done; }
    if ((result = avcodec_parameters_to_context(decoder, input->streams[stream]->codecpar)) < 0)
        goto done;
    decoder->thread_count = 1;
    if ((result = avcodec_open2(decoder, codec, NULL)) < 0)
        goto done;
    while ((result = av_read_frame(input, packet)) >= 0) {
        if (++packets > 32) { result = AVERROR_INVALIDDATA; goto done; }
        if (packet->stream_index == stream) {
            if ((result = avcodec_send_packet(decoder, packet)) < 0 ||
                (result = receive_frames(decoder, frame, hashes, &count)) < 0)
                goto done;
        }
        av_packet_unref(packet);
    }
    if (result != AVERROR_EOF ||
        (result = avcodec_send_packet(decoder, NULL)) < 0 ||
        (result = receive_frames(decoder, frame, hashes, &count)) < 0)
        goto done;
    int distinct = 0;
    for (int i = 0; i < count; ++i) {
        int seen = 0;
        for (int j = 0; j < i; ++j)
            seen |= hashes[i] == hashes[j];
        distinct += !seen;
    }
    if (count != 8 || distinct != 8) {
        fprintf(stderr, "Software AV1 motion failed: %d frames, %d distinct\n", count, distinct);
        result = AVERROR_INVALIDDATA;
        goto done;
    }
    printf("Software AV1 decode PASS: decoder=%s frames=%d distinct=%d size=64x64 format=yuv420p\n",
           codec->name, count, distinct);
    result = 0;
done:
    if (result < 0) {
        char error[AV_ERROR_MAX_STRING_SIZE];
        av_strerror(result, error, sizeof(error));
        fprintf(stderr, "Software AV1 decode failed: %s\n", error);
    }
    av_frame_free(&frame);
    av_packet_free(&packet);
    avcodec_free_context(&decoder);
    avformat_close_input(&input);
    return result < 0 ? 1 : 0;
}
